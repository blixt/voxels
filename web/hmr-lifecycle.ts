/**
 * Gives the Rust worker a bounded opportunity to close sockets and persistence before a Vite
 * reload, then guarantees that the old worker cannot survive into the replacement page.
 */
export async function terminateAfterAcknowledgement(
  acknowledgement: Promise<void>,
  terminate: () => void,
  timeoutMs = 1_000,
): Promise<void> {
  try {
    await Promise.race([
      acknowledgement,
      new Promise<void>((resolve) => setTimeout(resolve, timeoutMs)),
    ]);
  } finally {
    terminate();
  }
}
