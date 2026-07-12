interface ResolutionQuery {
  addEventListener(type: "change", listener: () => void): void;
  removeEventListener(type: "change", listener: () => void): void;
}

/// Re-arms a resolution media query whenever the device pixel ratio changes. A fixed query only
/// fires once when its old ratio stops matching, so each change must install a query for the new
/// ratio before notifying the renderer.
export function watchDevicePixelRatio(
  readPixelRatio: () => number,
  matchResolution: (query: string) => ResolutionQuery,
  onChange: () => void,
): () => void {
  let media: ResolutionQuery | undefined;
  const handleChange = (): void => {
    arm();
    onChange();
  };
  const arm = (): void => {
    media?.removeEventListener("change", handleChange);
    media = matchResolution(`(resolution: ${readPixelRatio()}dppx)`);
    media.addEventListener("change", handleChange);
  };
  arm();
  return () => media?.removeEventListener("change", handleChange);
}
