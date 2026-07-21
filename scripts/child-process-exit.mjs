export function mirrorChildExit(child) {
  let spawnFailed = false;

  child.on("error", (error) => {
    spawnFailed = true;
    console.error(error);
    process.exitCode = 1;
  });
  child.on("close", (code, signal) => {
    // Node reports an OS spawn error twice: first as `error`, then as `close`
    // with the negative errno in `code`. Keep the conventional failure status
    // from the error handler instead of wrapping that errno to (for example)
    // shell exit 254 for ENOENT.
    if (spawnFailed) {
      return;
    }
    if (code !== null) {
      process.exitCode = code;
    } else if (signal) {
      process.kill(process.pid, signal);
    } else {
      process.exitCode = 1;
    }
  });
}
