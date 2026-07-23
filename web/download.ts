export function downloadBlob(
  blob: Blob,
  requestedFilename: string,
  documentRef: Document = document,
  urlRef: typeof URL = URL,
): boolean {
  if (blob.size === 0) return false;
  const filename = requestedFilename.replace(/[^a-zA-Z0-9._-]/g, "_") || "voxels.png";
  let objectUrl: string | undefined;
  try {
    objectUrl = urlRef.createObjectURL(blob);
    const anchor = documentRef.createElement("a");
    anchor.href = objectUrl;
    anchor.download = filename;
    anchor.rel = "noopener";
    anchor.click();
    return true;
  } catch {
    return false;
  } finally {
    if (objectUrl !== undefined) {
      const urlToRevoke = objectUrl;
      setTimeout(() => urlRef.revokeObjectURL(urlToRevoke), 1_000);
    }
  }
}
