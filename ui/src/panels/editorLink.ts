// The link that opens the selected file in the person's editor. The daemon builds and checks it;
// the page shows it only when it still parses as a URL whose scheme is an editor's own, never a
// web, script, data or file address, so a link that went wrong can never load or run anything.

/** Schemes an editor link may never have. */
const REFUSED_SCHEMES: readonly string[] = ['http', 'https', 'javascript', 'data', 'file'];

/** The address to link to for `url`, or null when there is none or it may not be followed. */
export function editorHref(url: string | null): string | null {
  if (url === null) return null;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  const scheme = parsed.protocol.slice(0, -1).toLowerCase();
  return REFUSED_SCHEMES.includes(scheme) ? null : parsed.href;
}
