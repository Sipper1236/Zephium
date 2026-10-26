#![deny(clippy::dbg_macro, clippy::print_stderr, clippy::print_stdout)]
#![cfg_attr(
    not(test),
    deny(clippy::panic, clippy::unreachable, clippy::unwrap_used)
)]

//! Platform-neutral native navigation gate for owned agent contexts.
//!
//! Wry exposes the same identity-bearing navigation event contract on WebKit
//! and WebView2. Keeping the authority state machine here prevents the two
//! platform adapters from drifting on bootstrap, redirect, timeout, recovery,
//! and renderer-loss semantics.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use zephium_agentic::{
    ContextNavigationRedirectPolicy, ContextNavigationTarget, ContextOperationJoin,
    ContextOperationKind, ContextPortFailure, MAX_CONTEXT_NAVIGATION_REDIRECTS,
};

/// Closed committed target for one exact native page load.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgentNavigationCommit {
    /// A validated HTTP(S) document committed.
    Web(ContextNavigationTarget),
    /// The construction/recovery-only empty document committed.
    Bootstrap,
}

/// One exact terminal native observation for a shell-requested page load.
pub(crate) struct AgentNavigationTerminal {
    operation: ContextOperationJoin,
    outcome: Result<AgentNavigationCommit, ContextPortFailure>,
}

impl AgentNavigationTerminal {
    pub(crate) const fn operation(&self) -> ContextOperationJoin {
        self.operation
    }

    pub(crate) fn into_outcome(self) -> Result<AgentNavigationCommit, ContextPortFailure> {
        self.outcome
    }
}

pub(crate) struct AgentNavigationObservation {
    document_committed: bool,
    location_check_requested: bool,
    terminal: Option<AgentNavigationTerminal>,
}

impl AgentNavigationObservation {
    const fn none() -> Self {
        Self {
            document_committed: false,
            location_check_requested: false,
            terminal: None,
        }
    }

    const fn document_committed() -> Self {
        Self {
            document_committed: true,
            location_check_requested: false,
            terminal: None,
        }
    }

    fn terminal(_document_committed: bool, terminal: AgentNavigationTerminal) -> Self {
        Self {
            document_committed: _document_committed,
            location_check_requested: false,
            terminal: Some(terminal),
        }
    }

    const fn location_check_requested() -> Self {
        Self {
            document_committed: false,
            location_check_requested: true,
            terminal: None,
        }
    }

    pub(crate) const fn did_commit_document(&self) -> bool {
        self.document_committed
    }

    pub(crate) const fn should_check_location(&self) -> bool {
        self.location_check_requested
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) fn into_terminal(self) -> Option<AgentNavigationTerminal> {
        self.terminal
    }

    #[cfg(test)]
    pub(crate) fn is_none(&self) -> bool {
        self.terminal.is_none()
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn is_some(&self) -> bool {
        self.terminal.is_some()
    }

    #[cfg(test)]
    pub(crate) fn expect(self, message: &str) -> AgentNavigationTerminal {
        self.terminal.expect(message)
    }
}

/// Content-free redirect result visible only to the non-shipping native
/// qualification feature.
#[cfg(all(
    feature = "native-agentic-semantic-probe",
    any(test, target_os = "windows")
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AgentRedirectProbeAudit {
    redirects_observed: u8,
    limit_refused: bool,
}

#[cfg(all(
    feature = "native-agentic-semantic-probe",
    any(test, target_os = "windows")
))]
impl AgentRedirectProbeAudit {
    pub(crate) const fn redirects_observed(self) -> u8 {
        self.redirects_observed
    }

    pub(crate) const fn limit_refused(self) -> bool {
        self.limit_refused
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use zephium_agentic::{
        ContextCapabilities, ContextId, ContextIdentity, ContextOperationId, ContextRegistry,
        ContextRunId,
    };
    use zephium_core::ids::ProfileId;

    fn operation(
        kind: zephium_agentic::ContextOperationKind,
    ) -> zephium_agentic::ContextOperationJoin {
        let identity = ContextIdentity::new(
            ContextId::generate(),
            ContextRunId::generate(),
            ProfileId::from(44),
            zephium_agentic::ContextKind::Owned,
        );
        let capability = match kind {
            zephium_agentic::ContextOperationKind::Navigate => {
                zephium_agentic::ContextCapability::Navigate
            }
            zephium_agentic::ContextOperationKind::Recover => {
                zephium_agentic::ContextCapability::Recover
            }
            _ => panic!("test helper supports only page-load operations"),
        };
        let capabilities =
            ContextCapabilities::try_new(zephium_agentic::ContextKind::Owned, &[capability])
                .expect("capabilities");
        let mut registry = ContextRegistry::new();
        registry.reserve(identity, capabilities).expect("reserve");
        let construction = registry
            .begin_context(
                identity.id(),
                ContextOperationId::new(1).expect("operation"),
            )
            .expect("construction");
        registry
            .settle_construction(
                identity.id(),
                construction,
                zephium_agentic::ContextSettlement::Applied,
            )
            .expect("settle construction");
        match kind {
            zephium_agentic::ContextOperationKind::Navigate => registry
                .begin_navigation(
                    identity.id(),
                    ContextOperationId::new(2).expect("operation"),
                )
                .expect("navigation"),
            zephium_agentic::ContextOperationKind::Recover => {
                let prior = registry.join(identity.id()).expect("join");
                registry
                    .renderer_lost(identity.id(), prior)
                    .expect("renderer loss");
                registry
                    .begin_recovery(
                        identity.id(),
                        ContextOperationId::new(2).expect("operation"),
                    )
                    .expect("recovery")
            }
            _ => unreachable!(),
        }
    }

    fn event(id: u64, phase: wry::NavigationEventPhase, url: &str) -> wry::NavigationEvent {
        wry::NavigationEvent {
            id: wry::NavigationId::from_raw(id),
            phase,
            url: url.to_owned(),
        }
    }

    #[test]
    fn bootstrap_is_one_shot_and_never_emits_a_shell_terminal() {
        let gate = super::AgentNavigationController::default();
        assert!(gate.allows("about:blank"));
        assert!(!gate.allows("about:blank"));
        assert!(!gate.allows("https://example.test/"));
        assert!(gate
            .observe(event(70, wry::NavigationEventPhase::Started, "about:blank",))
            .expect("bootstrap start")
            .is_none());
        let commit = gate
            .observe(event(
                70,
                wry::NavigationEventPhase::Committed,
                "about:blank",
            ))
            .expect("bootstrap commit");
        assert!(commit.did_commit_document());
        assert!(commit.is_none());
    }

    #[test]
    fn native_cancellation_settles_only_its_exact_navigation_once() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let target =
            zephium_agentic::ContextNavigationTarget::parse("https://example.test/exact").unwrap();
        gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        gate.observe(event(
            8,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .unwrap();
        assert!(gate
            .observe(event(
                9,
                wry::NavigationEventPhase::Cancelled,
                target.as_url().as_str()
            ))
            .unwrap()
            .is_none());
        let terminal = gate
            .observe(event(
                8,
                wry::NavigationEventPhase::Cancelled,
                target.as_url().as_str(),
            ))
            .unwrap()
            .expect("one cancellation terminal");
        assert_eq!(terminal.operation(), operation);
        assert_eq!(
            terminal.into_outcome(),
            Err(zephium_agentic::ContextPortFailure::NativeRefused)
        );
        assert!(gate
            .observe(event(
                8,
                wry::NavigationEventPhase::Committed,
                target.as_url().as_str()
            ))
            .unwrap()
            .is_none());
    }

    #[test]
    fn armed_navigation_matches_exact_target_and_native_id_once() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let target = zephium_agentic::ContextNavigationTarget::parse("https://example.test/exact")
            .expect("target");
        gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
            .expect("arm");
        assert!(gate.allows(target.as_url().as_str()));
        assert!(!gate.allows("https://example.test/redirect"));
        assert!(gate
            .observe(event(
                8,
                wry::NavigationEventPhase::Started,
                target.as_url().as_str(),
            ))
            .expect("start")
            .is_none());
        assert!(gate
            .observe(event(
                9,
                wry::NavigationEventPhase::Committed,
                target.as_url().as_str(),
            ))
            .expect("wrong native id")
            .is_none());
        let terminal = gate
            .observe(event(
                8,
                wry::NavigationEventPhase::Committed,
                target.as_url().as_str(),
            ))
            .expect("commit")
            .expect("terminal");
        assert_eq!(terminal.operation(), operation);
        assert_eq!(
            terminal.into_outcome(),
            Ok(super::AgentNavigationCommit::Web(target))
        );
        assert!(gate
            .observe(event(
                8,
                wry::NavigationEventPhase::Failed,
                "https://example.test/exact",
            ))
            .expect("late failure")
            .is_none());
    }

    #[test]
    fn redirect_scope_commits_the_authoritative_final_target_under_one_native_id() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let requested = zephium_agentic::ContextNavigationTarget::parse("https://start.test/path")
            .expect("requested");
        let final_target =
            zephium_agentic::ContextNavigationTarget::parse("https://final.test/landing")
                .expect("final");
        let policy = zephium_agentic::ContextNavigationRedirectPolicy::try_new(vec![
            zephium_agentic::SemanticOrigin::parse("https://final.test/").expect("origin"),
        ])
        .expect("policy");
        gate.arm_with_redirect_policy(
            operation,
            requested.clone(),
            policy,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("arm");

        assert!(gate.allows(requested.as_url().as_str()));
        assert!(gate.allows(final_target.as_url().as_str()));
        assert!(!gate.allows("https://outside.test/"));
        gate.observe(event(
            81,
            wry::NavigationEventPhase::Started,
            requested.as_url().as_str(),
        ))
        .expect("start");
        assert!(gate
            .observe(event(
                81,
                wry::NavigationEventPhase::Redirected,
                final_target.as_url().as_str(),
            ))
            .expect("redirect")
            .is_none());
        let terminal = gate
            .observe(event(
                81,
                wry::NavigationEventPhase::Committed,
                final_target.as_url().as_str(),
            ))
            .expect("commit")
            .expect("terminal");
        #[cfg(feature = "native-agentic-semantic-probe")]
        {
            let audit = gate.redirect_probe_audit(operation).expect("probe audit");
            assert_eq!(audit.redirects_observed(), 1);
            assert!(!audit.limit_refused());
        }
        assert_eq!(
            terminal.into_outcome(),
            Ok(super::AgentNavigationCommit::Web(final_target.clone()))
        );
        assert!(gate.disarm(operation));
        gate.observe(event(
            81,
            wry::NavigationEventPhase::Finished,
            final_target.as_url().as_str(),
        ))
        .expect("finished");
        assert_eq!(gate.document_finished_for_audit(operation), Some(true));
    }

    #[test]
    fn macos_style_redirect_observations_use_identity_and_final_commit_url() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let requested = zephium_agentic::ContextNavigationTarget::parse("https://start.test/path")
            .expect("requested");
        let final_target =
            zephium_agentic::ContextNavigationTarget::parse("https://final.test/landing")
                .expect("final");
        let policy = zephium_agentic::ContextNavigationRedirectPolicy::try_new(vec![
            zephium_agentic::SemanticOrigin::parse("https://final.test/").expect("origin"),
        ])
        .expect("policy");
        gate.arm_with_redirect_policy(
            operation,
            requested.clone(),
            policy,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("arm");
        gate.observe(event(
            82,
            wry::NavigationEventPhase::Started,
            requested.as_url().as_str(),
        ))
        .expect("start");
        for _ in 0..2 {
            assert!(gate
                .observe(event(
                    82,
                    wry::NavigationEventPhase::Redirected,
                    requested.as_url().as_str(),
                ))
                .expect("opaque redirect")
                .is_none());
        }
        let terminal = gate
            .observe(event(
                82,
                wry::NavigationEventPhase::Committed,
                final_target.as_url().as_str(),
            ))
            .expect("commit")
            .expect("terminal");
        assert_eq!(
            terminal.into_outcome(),
            Ok(super::AgentNavigationCommit::Web(final_target))
        );
    }

    #[test]
    fn redirect_without_scope_or_beyond_hard_limit_is_refused_once() {
        let gate = super::AgentNavigationController::default();
        let exact_operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let target = zephium_agentic::ContextNavigationTarget::parse("https://example.test/start")
            .expect("target");
        gate.arm(
            exact_operation,
            target.clone(),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("arm");
        gate.observe(event(
            83,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .expect("start");
        let refusal = gate
            .observe(event(
                83,
                wry::NavigationEventPhase::Redirected,
                target.as_url().as_str(),
            ))
            .expect("redirect")
            .expect("terminal");
        assert_eq!(
            refusal.into_outcome(),
            Err(zephium_agentic::ContextPortFailure::NativeRefused)
        );
        assert!(gate
            .observe(event(
                83,
                wry::NavigationEventPhase::Committed,
                target.as_url().as_str(),
            ))
            .expect("late commit")
            .is_none());
        assert!(gate.disarm(exact_operation));

        let bounded = super::AgentNavigationController::default();
        let bounded_operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let policy =
            zephium_agentic::ContextNavigationRedirectPolicy::same_origin(&target).expect("policy");
        bounded
            .arm_with_redirect_policy(
                bounded_operation,
                target.clone(),
                policy,
                Arc::new(AtomicBool::new(false)),
            )
            .expect("bounded arm");
        bounded
            .observe(event(
                84,
                wry::NavigationEventPhase::Started,
                target.as_url().as_str(),
            ))
            .expect("bounded start");
        for hop in 0..zephium_agentic::MAX_CONTEXT_NAVIGATION_REDIRECTS {
            assert!(bounded
                .observe(event(
                    84,
                    wry::NavigationEventPhase::Redirected,
                    &format!("https://example.test/hop-{hop}"),
                ))
                .expect("bounded redirect")
                .is_none());
        }
        let overflow = bounded
            .observe(event(
                84,
                wry::NavigationEventPhase::Redirected,
                "https://example.test/overflow",
            ))
            .expect("overflow")
            .expect("overflow terminal");
        #[cfg(feature = "native-agentic-semantic-probe")]
        {
            let audit = bounded
                .redirect_probe_audit(bounded_operation)
                .expect("probe audit");
            assert_eq!(
                audit.redirects_observed(),
                zephium_agentic::MAX_CONTEXT_NAVIGATION_REDIRECTS as u8
            );
            assert!(audit.limit_refused());
        }
        assert_eq!(
            overflow.into_outcome(),
            Err(zephium_agentic::ContextPortFailure::NativeRefused)
        );
    }

    #[test]
    fn changed_commit_without_identity_bearing_redirect_is_refused() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let requested =
            zephium_agentic::ContextNavigationTarget::parse("https://example.test/start")
                .expect("requested");
        let policy = zephium_agentic::ContextNavigationRedirectPolicy::same_origin(&requested)
            .expect("policy");
        gate.arm_with_redirect_policy(
            operation,
            requested.clone(),
            policy,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("arm");
        gate.observe(event(
            85,
            wry::NavigationEventPhase::Started,
            requested.as_url().as_str(),
        ))
        .expect("start");
        let terminal = gate
            .observe(event(
                85,
                wry::NavigationEventPhase::Committed,
                "https://example.test/unobserved",
            ))
            .expect("commit")
            .expect("terminal");
        assert_eq!(
            terminal.into_outcome(),
            Err(zephium_agentic::ContextPortFailure::NativeRefused)
        );
    }

    #[test]
    fn committed_navigation_retains_one_content_free_finished_fact() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let target =
            zephium_agentic::ContextNavigationTarget::parse("https://example.test/finished")
                .expect("target");
        gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
            .expect("arm");
        assert!(gate.allows(target.as_url().as_str()));
        gate.observe(event(
            12,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .expect("start");
        gate.observe(event(
            12,
            wry::NavigationEventPhase::Committed,
            target.as_url().as_str(),
        ))
        .expect("commit")
        .expect("terminal");
        assert_eq!(gate.document_finished_for_audit(operation), Some(false));
        assert!(gate.disarm(operation));
        assert_eq!(gate.document_finished_for_audit(operation), Some(false));
        gate.observe(event(
            12,
            wry::NavigationEventPhase::Finished,
            target.as_url().as_str(),
        ))
        .expect("finished");
        assert_eq!(gate.document_finished_for_audit(operation), Some(true));
        assert!(gate
            .observe(event(
                12,
                wry::NavigationEventPhase::Finished,
                target.as_url().as_str(),
            ))
            .is_err());
    }

    #[test]
    fn timeout_claim_blocks_a_late_native_commit() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let target = zephium_agentic::ContextNavigationTarget::parse("https://example.test/late")
            .expect("target");
        let claimed = Arc::new(AtomicBool::new(false));
        gate.arm(operation, target.clone(), claimed.clone())
            .expect("arm");
        gate.observe(event(
            10,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .expect("start");
        assert!(!claimed.swap(true, Ordering::AcqRel));
        assert!(gate
            .observe(event(
                10,
                wry::NavigationEventPhase::Committed,
                target.as_url().as_str(),
            ))
            .expect("late commit")
            .is_none());
        assert!(gate.disarm(operation));
    }

    #[test]
    fn recovery_requires_loss_and_failed_recovery_restores_loss() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Recover);
        let target =
            zephium_agentic::ContextNavigationTarget::parse("https://example.test/recover")
                .expect("target");
        assert!(gate
            .arm_recovery(
                operation,
                Some(target.clone()),
                Arc::new(AtomicBool::new(false)),
            )
            .is_err());
        assert_eq!(gate.claim_renderer_loss(), Ok(true));
        assert_eq!(gate.claim_renderer_loss(), Ok(false));
        gate.arm_recovery(
            operation,
            Some(target.clone()),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("recovery arm");
        assert!(gate.allows(target.as_url().as_str()));
        assert!(!gate.allows("https://example.test/redirect"));
        gate.observe(event(
            11,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .expect("start");
        let terminal = gate
            .observe(event(
                11,
                wry::NavigationEventPhase::Failed,
                target.as_url().as_str(),
            ))
            .expect("failure")
            .expect("terminal");
        assert_eq!(
            terminal.into_outcome(),
            Err(zephium_agentic::ContextPortFailure::NativeRefused)
        );
        assert!(gate.settle_recovery(operation, false));
        assert!(gate.matches_for_audit(None, true));
    }

    #[test]
    fn finished_document_coalesces_location_signals_until_exact_rejoin() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let target =
            zephium_agentic::ContextNavigationTarget::parse("https://example.test/history-start")
                .expect("target");
        gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
            .expect("arm");
        assert!(gate.allows(target.as_url().as_str()));
        gate.observe(event(
            31,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .expect("start");
        gate.observe(event(
            31,
            wry::NavigationEventPhase::Committed,
            target.as_url().as_str(),
        ))
        .expect("commit")
        .expect("terminal");

        // A page can mutate history after commit but before the native load
        // completion. Retain only one content-free dirty fact until Finished.
        assert_eq!(gate.request_location_check(), Ok(false));
        assert_eq!(gate.request_location_check(), Ok(false));
        assert!(gate.disarm(operation));
        let finished = gate
            .observe(event(
                31,
                wry::NavigationEventPhase::Finished,
                target.as_url().as_str(),
            ))
            .expect("finished");
        assert!(finished.should_check_location());
        assert_eq!(gate.location_state_for_audit(), Some((true, false, false)));

        // A hostile History API loop cannot enqueue more callbacks while the
        // sole check is owned, nor while its replacement awaits shell rejoin.
        assert_eq!(gate.request_location_check(), Ok(false));
        assert_eq!(gate.finish_location_check(true), Ok(()));
        assert_eq!(gate.location_state_for_audit(), Some((false, true, true)));
        assert_eq!(gate.request_location_check(), Ok(false));
        assert_eq!(gate.acknowledge_location_replacement(), Ok(true));
        assert_eq!(gate.location_state_for_audit(), Some((true, false, false)));
        assert_eq!(gate.finish_location_check(false), Ok(()));
        assert_eq!(gate.location_state_for_audit(), Some((false, false, false)));
    }

    #[test]
    fn committed_is_not_ready_and_revocation_fences_late_native_failure() {
        for sealed in [false, true] {
            let gate = super::AgentNavigationController::default();
            let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
            let target =
                zephium_agentic::ContextNavigationTarget::parse("https://example.test/ready")
                    .expect("target");
            gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
                .expect("arm");
            gate.observe(event(
                51,
                wry::NavigationEventPhase::Started,
                target.as_url().as_str(),
            ))
            .expect("start");
            gate.observe(event(
                51,
                wry::NavigationEventPhase::Committed,
                target.as_url().as_str(),
            ))
            .expect("commit");
            assert!(gate.disarm(operation));
            assert!(!gate.location_stable_for_result());
            assert_eq!(gate.document_finished_for_audit(operation), Some(false));
            if sealed {
                assert!(gate.seal_location_observation());
                assert!(!gate.allows(target.as_url().as_str()));
                assert_eq!(gate.request_location_check(), Ok(false));
                assert!(gate
                    .arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
                    .is_err());
            }
            let late = gate.observe(event(
                51,
                wry::NavigationEventPhase::Failed,
                target.as_url().as_str(),
            ));
            assert_eq!(
                late.is_ok(),
                sealed,
                "only explicit revocation can fence the callback"
            );
            assert_eq!(gate.document_finished_for_audit(operation), Some(false));
            assert!(!gate.location_stable_for_result());
        }
    }

    #[cfg(all(target_os = "macos", feature = "native-agentic-foreground-probe"))]
    #[test]
    fn foreground_readiness_distinguishes_exact_commit_finish_and_location_reconciliation() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let context = operation.context();
        let target =
            zephium_agentic::ContextNavigationTarget::parse("https://example.test/render").unwrap();
        assert_eq!(gate.rendering_document_ready(context), None);
        gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
            .unwrap();
        gate.observe(event(
            71,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .unwrap();
        let committed = gate
            .observe(event(
                71,
                wry::NavigationEventPhase::Committed,
                target.as_url().as_str(),
            ))
            .unwrap()
            .expect("commit terminal");
        assert!(committed.into_outcome().is_ok());
        assert_eq!(
            gate.rendering_document_ready(context),
            None,
            "unsettled arm is not a wait"
        );
        assert!(gate.disarm(operation));
        assert_eq!(gate.rendering_document_ready(context), Some(false));
        assert!(!gate.location_stable_for_result());
        assert_eq!(gate.request_location_check(), Ok(false));
        gate.observe(event(
            72,
            wry::NavigationEventPhase::Finished,
            target.as_url().as_str(),
        ))
        .unwrap();
        assert_eq!(
            gate.rendering_document_ready(context),
            Some(false),
            "foreign finish is not readiness"
        );
        gate.observe(event(
            71,
            wry::NavigationEventPhase::Finished,
            target.as_url().as_str(),
        ))
        .unwrap();
        assert_eq!(
            gate.rendering_document_ready(context),
            Some(false),
            "location receipt still owns reconciliation"
        );
        gate.finish_location_check(false).unwrap();
        assert_eq!(gate.rendering_document_ready(context), Some(true));
        assert!(gate.location_stable_for_result());
        let foreign = self::operation(zephium_agentic::ContextOperationKind::Navigate).context();
        assert_eq!(gate.rendering_document_ready(foreign), None);
        gate.request_location_check().unwrap();
        gate.finish_location_check(true).unwrap();
        assert_eq!(
            gate.rendering_document_ready(context),
            None,
            "replacement cannot become a wait"
        );
    }

    #[cfg(all(target_os = "macos", feature = "native-agentic-foreground-probe"))]
    #[test]
    fn foreground_readiness_never_waits_through_cancellation_or_renderer_loss() {
        for cancel in [true, false] {
            let gate = super::AgentNavigationController::default();
            let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
            let target =
                zephium_agentic::ContextNavigationTarget::parse("https://example.test/render")
                    .unwrap();
            gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
                .unwrap();
            gate.observe(event(
                81,
                wry::NavigationEventPhase::Started,
                target.as_url().as_str(),
            ))
            .unwrap();
            gate.observe(event(
                81,
                wry::NavigationEventPhase::Committed,
                target.as_url().as_str(),
            ))
            .unwrap();
            assert!(gate.disarm(operation));
            assert_eq!(
                gate.rendering_document_ready(operation.context()),
                Some(false)
            );
            if cancel {
                assert!(gate.seal_location_observation());
            } else {
                assert_eq!(gate.claim_renderer_loss(), Ok(true));
            }
            gate.observe(event(
                81,
                wry::NavigationEventPhase::Finished,
                target.as_url().as_str(),
            ))
            .unwrap();
            assert_eq!(gate.rendering_document_ready(operation.context()), None);
        }
    }

    #[test]
    fn location_check_can_defer_without_creating_a_second_queue_entry() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let target =
            zephium_agentic::ContextNavigationTarget::parse("https://example.test/deferred")
                .expect("target");
        gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
            .expect("arm");
        assert!(gate.allows(target.as_url().as_str()));
        gate.observe(event(
            32,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .expect("start");
        gate.observe(event(
            32,
            wry::NavigationEventPhase::Committed,
            target.as_url().as_str(),
        ))
        .expect("commit")
        .expect("terminal");
        assert!(gate.disarm(operation));
        gate.observe(event(
            32,
            wry::NavigationEventPhase::Finished,
            target.as_url().as_str(),
        ))
        .expect("finished");

        assert_eq!(gate.request_location_check(), Ok(true));
        assert_eq!(gate.defer_location_check(), Ok(()));
        assert_eq!(gate.location_state_for_audit(), Some((false, true, false)));
        assert_eq!(gate.request_deferred_location_check(), Ok(true));
        assert_eq!(gate.finish_location_check(false), Ok(()));
        assert_eq!(gate.location_state_for_audit(), Some((false, false, false)));
    }

    #[test]
    fn renderer_loss_preserves_only_an_emitted_rejoin_barrier() {
        let gate = super::AgentNavigationController::default();
        let operation = operation(zephium_agentic::ContextOperationKind::Navigate);
        let target = zephium_agentic::ContextNavigationTarget::parse(
            "https://example.test/replacement-before-loss",
        )
        .expect("target");
        gate.arm(operation, target.clone(), Arc::new(AtomicBool::new(false)))
            .expect("arm");
        assert!(gate.allows(target.as_url().as_str()));
        gate.observe(event(
            33,
            wry::NavigationEventPhase::Started,
            target.as_url().as_str(),
        ))
        .expect("start");
        gate.observe(event(
            33,
            wry::NavigationEventPhase::Committed,
            target.as_url().as_str(),
        ))
        .expect("commit")
        .expect("terminal");
        assert!(gate.disarm(operation));
        gate.observe(event(
            33,
            wry::NavigationEventPhase::Finished,
            target.as_url().as_str(),
        ))
        .expect("finished");
        assert_eq!(gate.request_location_check(), Ok(true));
        assert_eq!(gate.finish_location_check(true), Ok(()));

        assert_eq!(gate.claim_renderer_loss(), Ok(true));
        assert_eq!(gate.location_state_for_audit(), Some((false, false, true)));
        assert_eq!(gate.acknowledge_location_replacement(), Ok(false));
        assert_eq!(gate.location_state_for_audit(), Some((false, false, false)));
        assert!(!gate.location_stable_for_result());
    }
}

#[derive(Clone)]
enum AgentLoadExpectation {
    Web {
        requested: ContextNavigationTarget,
        redirects: Option<ContextNavigationRedirectPolicy>,
    },
    Bootstrap,
}

impl AgentLoadExpectation {
    fn matches_initial(&self, candidate: &str) -> bool {
        match self {
            Self::Web { requested, .. } => ContextNavigationTarget::parse(candidate)
                .ok()
                .is_some_and(|candidate| candidate == *requested),
            Self::Bootstrap => candidate == "about:blank",
        }
    }

    fn allows(&self, candidate: &str) -> bool {
        match self {
            Self::Web {
                requested,
                redirects,
            } => ContextNavigationTarget::parse(candidate)
                .ok()
                .is_some_and(|candidate| {
                    candidate == *requested
                        || redirects
                            .as_ref()
                            .is_some_and(|policy| policy.allows(&candidate))
                }),
            Self::Bootstrap => candidate == "about:blank",
        }
    }

    fn allows_observed_redirect(&self, candidate: &str) -> bool {
        match self {
            Self::Web {
                requested,
                redirects: Some(redirects),
            } => ContextNavigationTarget::parse(candidate)
                .ok()
                .is_some_and(|candidate| candidate == *requested || redirects.allows(&candidate)),
            Self::Web {
                redirects: None, ..
            }
            | Self::Bootstrap => false,
        }
    }

    fn commit(
        &self,
        candidate: &str,
        redirects_observed: usize,
    ) -> Result<AgentNavigationCommit, ContextPortFailure> {
        match self {
            Self::Web {
                requested,
                redirects,
            } => {
                let candidate = ContextNavigationTarget::parse(candidate)
                    .map_err(|_| ContextPortFailure::NativeRefused)?;
                let exact_without_redirect = redirects_observed == 0 && candidate == *requested;
                let allowed_redirect = redirects_observed > 0
                    && (candidate == *requested
                        || redirects
                            .as_ref()
                            .is_some_and(|policy| policy.allows(&candidate)));
                (exact_without_redirect || allowed_redirect)
                    .then_some(AgentNavigationCommit::Web(candidate))
                    .ok_or(ContextPortFailure::NativeRefused)
            }
            Self::Bootstrap if candidate == "about:blank" => Ok(AgentNavigationCommit::Bootstrap),
            Self::Bootstrap => Err(ContextPortFailure::NativeRefused),
        }
    }

    const fn is_web(&self) -> bool {
        matches!(self, Self::Web { .. })
    }
}

struct AgentNavigationArm {
    operation: ContextOperationJoin,
    expected: AgentLoadExpectation,
    terminal_claimed: Arc<AtomicBool>,
    native_id: Option<wry::NavigationId>,
    committed: bool,
    committed_target: Option<ContextNavigationTarget>,
    redirects_observed: usize,
    #[cfg(all(
        feature = "native-agentic-semantic-probe",
        any(test, target_os = "windows")
    ))]
    redirect_limit_refused: bool,
    finished: bool,
}

struct AgentCommittedNavigation {
    operation: ContextOperationJoin,
    native_id: wry::NavigationId,
    finished: bool,
    observes_web_location: bool,
}

struct AgentNavigationState {
    observation_sealed: bool,
    bootstrap_available: bool,
    bootstrap_pending: bool,
    bootstrap_native_id: Option<wry::NavigationId>,
    renderer_lost: bool,
    armed: Option<AgentNavigationArm>,
    last_committed: Option<AgentCommittedNavigation>,
    location_ready: bool,
    location_callback_pending: bool,
    location_dirty: bool,
    location_replacement_pending: bool,
}

impl Default for AgentNavigationState {
    fn default() -> Self {
        Self {
            observation_sealed: false,
            bootstrap_available: true,
            bootstrap_pending: false,
            bootstrap_native_id: None,
            renderer_lost: false,
            armed: None,
            last_committed: None,
            location_ready: false,
            location_callback_pending: false,
            location_dirty: false,
            location_replacement_pending: false,
        }
    }
}

/// Exact, one-at-a-time navigation policy shared with Wry's native delegates.
///
/// Unarmed page navigation is denied. The one bootstrap `about:blank` permit
/// exists solely for construction and is permanently consumed or sealed by
/// the first shell arm. Redirects remain denied by default; an explicit
/// trusted-shell scope admits only bounded destinations while native event
/// identity and hop observations retain attribution authority.
#[derive(Clone, Default)]
pub(crate) struct AgentNavigationController {
    state: Arc<Mutex<AgentNavigationState>>,
}

impl AgentNavigationController {
    pub(crate) fn arm(
        &self,
        operation: ContextOperationJoin,
        target: ContextNavigationTarget,
        terminal_claimed: Arc<AtomicBool>,
    ) -> Result<(), ()> {
        if operation.kind() != ContextOperationKind::Navigate {
            return Err(());
        }
        self.arm_exact(
            operation,
            AgentLoadExpectation::Web {
                requested: target,
                redirects: None,
            },
            terminal_claimed,
            false,
        )
    }

    /// Arms one navigation with immutable trusted-shell redirect authority.
    pub(crate) fn arm_with_redirect_policy(
        &self,
        operation: ContextOperationJoin,
        target: ContextNavigationTarget,
        redirects: ContextNavigationRedirectPolicy,
        terminal_claimed: Arc<AtomicBool>,
    ) -> Result<(), ()> {
        if operation.kind() != ContextOperationKind::Navigate {
            return Err(());
        }
        self.arm_exact(
            operation,
            AgentLoadExpectation::Web {
                requested: target,
                redirects: Some(redirects),
            },
            terminal_claimed,
            false,
        )
    }

    /// Arms the sole page load permitted after exact renderer loss.
    pub(crate) fn arm_recovery(
        &self,
        operation: ContextOperationJoin,
        target: Option<ContextNavigationTarget>,
        terminal_claimed: Arc<AtomicBool>,
    ) -> Result<(), ()> {
        if operation.kind() != ContextOperationKind::Recover {
            return Err(());
        }
        self.arm_exact(
            operation,
            target.map_or(AgentLoadExpectation::Bootstrap, |requested| {
                AgentLoadExpectation::Web {
                    requested,
                    redirects: None,
                }
            }),
            terminal_claimed,
            true,
        )
    }

    fn arm_exact(
        &self,
        operation: ContextOperationJoin,
        expected: AgentLoadExpectation,
        terminal_claimed: Arc<AtomicBool>,
        requires_renderer_loss: bool,
    ) -> Result<(), ()> {
        if terminal_claimed.load(Ordering::Acquire) {
            return Err(());
        }
        let mut state = self.state.lock().map_err(|_| ())?;
        if state.observation_sealed
            || state.renderer_lost != requires_renderer_loss
            || state.armed.is_some()
            || state.location_callback_pending
            || state.location_dirty
            || state.location_replacement_pending
        {
            return Err(());
        }
        state.bootstrap_available = false;
        state.bootstrap_pending = false;
        state.bootstrap_native_id = None;
        state.renderer_lost = false;
        state.last_committed = None;
        state.location_ready = false;
        state.armed = Some(AgentNavigationArm {
            operation,
            expected,
            terminal_claimed,
            native_id: None,
            committed: false,
            committed_target: None,
            redirects_observed: 0,
            #[cfg(all(
                feature = "native-agentic-semantic-probe",
                any(test, target_os = "windows")
            ))]
            redirect_limit_refused: false,
            finished: false,
        });
        Ok(())
    }

    pub(crate) fn disarm(&self, operation: ContextOperationJoin) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let Some(armed) = state.armed.take_if(|armed| armed.operation == operation) else {
            return false;
        };
        if !armed.committed {
            return true;
        }
        let Some(native_id) = armed.native_id else {
            return false;
        };
        state.last_committed = Some(AgentCommittedNavigation {
            operation,
            native_id,
            finished: armed.finished,
            observes_web_location: armed.expected.is_web(),
        });
        true
    }

    /// Retires one exact recovery arm and restores loss state on refusal.
    pub(crate) fn settle_recovery(&self, operation: ContextOperationJoin, applied: bool) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if operation.kind() != ContextOperationKind::Recover
            || !state
                .armed
                .as_ref()
                .is_some_and(|armed| armed.operation == operation)
        {
            return false;
        }
        let Some(armed) = state.armed.take() else {
            return false;
        };
        state.last_committed = if applied && armed.committed && !state.renderer_lost {
            armed.native_id.map(|native_id| AgentCommittedNavigation {
                operation,
                native_id,
                finished: armed.finished,
                observes_web_location: armed.expected.is_web(),
            })
        } else {
            None
        };
        // A termination callback can win after the commit callback has
        // claimed the terminal but before the host consumes either queued
        // callback. Successful settlement must not erase that newer loss.
        if !applied {
            state.renderer_lost = true;
        }
        true
    }

    pub(crate) fn allows(&self, candidate: &str) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.renderer_lost || state.observation_sealed {
            return false;
        }
        if let Some(armed) = state.armed.as_ref() {
            if armed.terminal_claimed.load(Ordering::Acquire) {
                return false;
            }
            return armed.expected.allows(candidate);
        }
        if state.bootstrap_available && candidate == "about:blank" {
            state.bootstrap_available = false;
            state.bootstrap_pending = true;
            return true;
        }
        false
    }

    pub(crate) fn observe(
        &self,
        event: wry::NavigationEvent,
    ) -> Result<AgentNavigationObservation, ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        if state.renderer_lost || state.observation_sealed {
            return Ok(AgentNavigationObservation::none());
        }
        let Some(armed) = state.armed.as_mut() else {
            if let Some(committed) = state.last_committed.as_mut() {
                if committed.native_id == event.id
                    && matches!(
                        event.phase,
                        wry::NavigationEventPhase::Finished
                            | wry::NavigationEventPhase::Failed
                            | wry::NavigationEventPhase::Cancelled
                    )
                {
                    if event.phase != wry::NavigationEventPhase::Finished || committed.finished {
                        return Err(());
                    }
                    committed.finished = true;
                    state.location_ready = committed.observes_web_location;
                    if state.location_ready
                        && state.location_dirty
                        && !state.location_callback_pending
                        && !state.location_replacement_pending
                    {
                        state.location_dirty = false;
                        state.location_callback_pending = true;
                        return Ok(AgentNavigationObservation::location_check_requested());
                    }
                    return Ok(AgentNavigationObservation::none());
                }
            }
            if !state.bootstrap_pending {
                return Ok(AgentNavigationObservation::none());
            }
            if event.phase == wry::NavigationEventPhase::Started {
                if event.url == "about:blank" && state.bootstrap_native_id.is_none() {
                    state.bootstrap_native_id = Some(event.id);
                }
                return Ok(AgentNavigationObservation::none());
            }
            if !matches!(
                event.phase,
                wry::NavigationEventPhase::Committed
                    | wry::NavigationEventPhase::Failed
                    | wry::NavigationEventPhase::Cancelled
            ) || state.bootstrap_native_id != Some(event.id)
            {
                return Ok(AgentNavigationObservation::none());
            }
            state.bootstrap_pending = false;
            state.bootstrap_native_id = None;
            return match event.phase {
                wry::NavigationEventPhase::Committed if event.url == "about:blank" => {
                    Ok(AgentNavigationObservation::document_committed())
                }
                wry::NavigationEventPhase::Failed | wry::NavigationEventPhase::Cancelled => {
                    Ok(AgentNavigationObservation::none())
                }
                wry::NavigationEventPhase::Committed => Err(()),
                wry::NavigationEventPhase::Started
                | wry::NavigationEventPhase::Redirected
                | wry::NavigationEventPhase::Finished => Ok(AgentNavigationObservation::none()),
            };
        };
        if event.phase == wry::NavigationEventPhase::Started {
            let matches_target = armed.expected.matches_initial(&event.url);
            if matches_target && armed.native_id.is_none() {
                armed.native_id = Some(event.id);
            }
            return Ok(AgentNavigationObservation::none());
        }
        if event.phase == wry::NavigationEventPhase::Redirected {
            if armed.native_id != Some(event.id) {
                return Ok(AgentNavigationObservation::none());
            }
            let redirect_is_allowed = !armed.committed
                && armed.redirects_observed < MAX_CONTEXT_NAVIGATION_REDIRECTS
                && armed.expected.allows_observed_redirect(&event.url);
            if redirect_is_allowed {
                armed.redirects_observed += 1;
                return Ok(AgentNavigationObservation::none());
            }
            if armed
                .terminal_claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Ok(AgentNavigationObservation::none());
            }
            #[cfg(all(
                feature = "native-agentic-semantic-probe",
                any(test, target_os = "windows")
            ))]
            {
                armed.redirect_limit_refused =
                    armed.redirects_observed >= MAX_CONTEXT_NAVIGATION_REDIRECTS;
            }
            return Ok(AgentNavigationObservation::terminal(
                false,
                AgentNavigationTerminal {
                    operation: armed.operation,
                    outcome: Err(ContextPortFailure::NativeRefused),
                },
            ));
        }
        if event.phase == wry::NavigationEventPhase::Finished && armed.native_id == Some(event.id) {
            let matches_committed = armed.committed_target.as_ref().map_or_else(
                || event.url == "about:blank",
                |target| {
                    ContextNavigationTarget::parse(&event.url)
                        .ok()
                        .is_some_and(|candidate| candidate == *target)
                },
            );
            if !armed.committed || !matches_committed || armed.finished {
                return Err(());
            }
            armed.finished = true;
            state.location_ready = armed.expected.is_web();
            if state.location_ready
                && state.location_dirty
                && !state.location_callback_pending
                && !state.location_replacement_pending
            {
                state.location_dirty = false;
                state.location_callback_pending = true;
                return Ok(AgentNavigationObservation::location_check_requested());
            }
            return Ok(AgentNavigationObservation::none());
        }
        if !matches!(
            event.phase,
            wry::NavigationEventPhase::Committed
                | wry::NavigationEventPhase::Failed
                | wry::NavigationEventPhase::Cancelled
        ) || armed.native_id != Some(event.id)
        {
            return Ok(AgentNavigationObservation::none());
        }
        if armed
            .terminal_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(AgentNavigationObservation::none());
        }
        let operation = armed.operation;
        let expected = armed.expected.clone();
        let outcome = match event.phase {
            wry::NavigationEventPhase::Committed => {
                expected.commit(&event.url, armed.redirects_observed)
            }
            // A cancelled navigation (a download handoff, a policy refusal)
            // ends without a document, exactly as a failed one does.
            wry::NavigationEventPhase::Failed | wry::NavigationEventPhase::Cancelled => {
                Err(ContextPortFailure::NativeRefused)
            }
            wry::NavigationEventPhase::Started
            | wry::NavigationEventPhase::Redirected
            | wry::NavigationEventPhase::Finished => return Ok(AgentNavigationObservation::none()),
        };
        let document_committed = outcome.is_ok();
        armed.committed = document_committed;
        armed.committed_target = match &outcome {
            Ok(AgentNavigationCommit::Web(target)) => Some(target.clone()),
            Ok(AgentNavigationCommit::Bootstrap) | Err(_) => None,
        };
        Ok(AgentNavigationObservation::terminal(
            document_committed,
            AgentNavigationTerminal { operation, outcome },
        ))
    }

    pub(crate) fn claim_renderer_loss(&self) -> Result<bool, ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        if state.renderer_lost {
            return Ok(false);
        }
        state.renderer_lost = true;
        state.last_committed = None;
        state.location_ready = false;
        state.location_callback_pending = false;
        state.location_dirty = false;
        if let Some(armed) = state.armed.as_ref() {
            armed.terminal_claimed.store(true, Ordering::Release);
        }
        Ok(true)
    }

    /// Claims the single bounded native-location callback for an idle,
    /// finished web document. Repeated platform signals collapse into one
    /// dirty bit until the host either samples the current URL or rejoins a
    /// replacement event.
    pub(crate) fn request_location_check(&self) -> Result<bool, ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        if state.renderer_lost || state.observation_sealed {
            return Ok(false);
        }
        if !state.location_ready {
            let committed_web_document = state
                .armed
                .as_ref()
                .is_some_and(|armed| armed.committed && armed.expected.is_web())
                || state
                    .last_committed
                    .as_ref()
                    .is_some_and(|committed| committed.observes_web_location);
            state.location_dirty |= committed_web_document;
            return Ok(false);
        }
        if state.location_callback_pending || state.location_replacement_pending {
            state.location_dirty = true;
            return Ok(false);
        }
        state.location_dirty = false;
        state.location_callback_pending = true;
        Ok(true)
    }

    /// Finishes one claimed URL sample. A detected replacement holds the
    /// observer closed until the shell proves the successor join on a later
    /// request. A no-change sample consumes every signal that preceded it.
    pub(crate) fn finish_location_check(&self, replacement: bool) -> Result<(), ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        if !state.location_callback_pending || !state.location_ready || state.renderer_lost {
            return Err(());
        }
        state.location_callback_pending = false;
        if replacement {
            if state.location_replacement_pending {
                return Err(());
            }
            state.location_replacement_pending = true;
        } else {
            state.location_dirty = false;
        }
        Ok(())
    }

    /// Defers a claimed sample while the host owns an exact lifecycle
    /// operation. The one dirty bit is retried only after that operation has
    /// reached a terminal state.
    pub(crate) fn defer_location_check(&self) -> Result<(), ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        if !state.location_callback_pending || state.renderer_lost {
            return Err(());
        }
        state.location_callback_pending = false;
        state.location_dirty = true;
        Ok(())
    }

    /// Claims a previously deferred dirty fact once the host is idle again.
    pub(crate) fn request_deferred_location_check(&self) -> Result<bool, ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        if state.renderer_lost
            || !state.location_ready
            || !state.location_dirty
            || state.location_callback_pending
            || state.location_replacement_pending
        {
            return Ok(false);
        }
        state.location_dirty = false;
        state.location_callback_pending = true;
        Ok(true)
    }

    /// Acknowledges the exact shell rejoin for the last emitted replacement.
    /// Returns true only when one coalesced later signal now owns the sole
    /// follow-up callback slot.
    pub(crate) fn acknowledge_location_replacement(&self) -> Result<bool, ()> {
        let mut state = self.state.lock().map_err(|_| ())?;
        if !state.location_replacement_pending {
            return Err(());
        }
        state.location_replacement_pending = false;
        if state.renderer_lost {
            return Ok(false);
        }
        if state.location_ready && state.location_dirty {
            state.location_dirty = false;
            state.location_callback_pending = true;
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn location_state_for_audit(&self) -> Option<(bool, bool, bool)> {
        self.state.lock().ok().map(|state| {
            (
                state.location_callback_pending,
                state.location_dirty,
                state.location_replacement_pending,
            )
        })
    }

    /// Revalidates that a semantic or visual result did not race any native
    /// location signal, replacement rejoin, page load, or renderer loss.
    #[cfg(any(
        target_os = "macos",
        test,
        all(target_os = "windows", feature = "native-agentic-semantic-probe")
    ))]
    pub(crate) fn location_stable_for_result(&self) -> bool {
        self.state.lock().is_ok_and(|state| {
            state.location_ready
                && !state.location_callback_pending
                && !state.location_dirty
                && !state.location_replacement_pending
                && !state.renderer_lost
                && state.armed.is_none()
        })
    }

    /// Closed readiness for the one release-excluded rendering acquisition.
    /// Commit alone is pending, not failure or render authority. An unknown,
    /// replaced, cancelled or lost exact document cannot become a wait/retry.
    #[cfg(all(target_os = "macos", feature = "native-agentic-foreground-probe"))]
    pub(crate) fn rendering_document_ready(
        &self,
        context: zephium_agentic::ContextJoin,
    ) -> Option<bool> {
        let state = self.state.lock().ok()?;
        if state.renderer_lost
            || state.observation_sealed
            || state.armed.is_some()
            || state.location_replacement_pending
        {
            return None;
        }
        let committed = state.last_committed.as_ref().filter(|committed| {
            committed.operation.context() == context && committed.observes_web_location
        })?;
        if !committed.finished {
            return (!state.location_ready && !state.location_callback_pending).then_some(false);
        }
        state
            .location_ready
            .then_some(!state.location_callback_pending && !state.location_dirty)
    }

    /// Permanently closes location observation for a cancelled or retiring
    /// context without granting or changing navigation authority.
    pub(crate) fn seal_location_observation(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        state.observation_sealed = true;
        state.location_ready = false;
        state.location_callback_pending = false;
        state.location_dirty = false;
        state.location_replacement_pending = false;
        true
    }

    /// Reports whether the exact committed navigation has reached native load
    /// completion. This is a read-only bounded audit seam; it grants no new
    /// navigation, script, page-content, or input authority.
    #[allow(dead_code)]
    pub(crate) fn document_finished_for_audit(
        &self,
        operation: ContextOperationJoin,
    ) -> Option<bool> {
        let state = self.state.lock().ok()?;
        if let Some(armed) = state
            .armed
            .as_ref()
            .filter(|armed| armed.operation == operation && armed.committed)
        {
            return Some(armed.finished);
        }
        state
            .last_committed
            .as_ref()
            .filter(|committed| committed.operation == operation)
            .map(|committed| committed.finished)
    }

    /// Returns content-free redirect facts only to the mechanically excluded
    /// native qualifier. Shipping builds retain neither the refusal bit nor
    /// this inspection surface.
    #[cfg(all(
        feature = "native-agentic-semantic-probe",
        any(test, target_os = "windows")
    ))]
    pub(crate) fn redirect_probe_audit(
        &self,
        operation: ContextOperationJoin,
    ) -> Option<AgentRedirectProbeAudit> {
        let state = self.state.lock().ok()?;
        let armed = state
            .armed
            .as_ref()
            .filter(|armed| armed.operation == operation)?;
        Some(AgentRedirectProbeAudit {
            redirects_observed: u8::try_from(armed.redirects_observed).ok()?,
            limit_refused: armed.redirect_limit_refused,
        })
    }

    pub(crate) fn matches_for_audit(
        &self,
        pending: Option<ContextOperationJoin>,
        renderer_lost: bool,
    ) -> bool {
        self.state.lock().is_ok_and(|state| {
            state.renderer_lost == renderer_lost
                && state.armed.as_ref().map(|armed| armed.operation) == pending
        })
    }
}
