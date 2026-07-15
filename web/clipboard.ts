export interface TextClipboard {
  writeText(text: string): Promise<void>;
}

/**
 * Copies diagnostics through the modern async clipboard API. Keeping the capability injectable
 * makes failures testable without adding a legacy DOM fallback to the canvas-only client.
 */
export async function writeClipboardText(
  clipboard: TextClipboard | undefined,
  text: string,
): Promise<boolean> {
  if (!clipboard) return false;
  try {
    await clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
