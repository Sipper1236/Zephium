// Keep unsupported hosts from failing later with opaque native build errors.
if (!["darwin", "win32", "linux"].includes(process.platform)) {
  console.error(
    `Zephium does not support ${process.platform}. Use macOS, Windows, or Linux.`,
  );
  process.exit(1);
}
