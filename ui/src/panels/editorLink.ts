// The link that opens the selected file in the person's editor. The daemon builds and checks it;
// the page shows it only when it still parses as a URL whose scheme is an editor's own, never a
// web, script, data or file address, so a link that went wrong can never load or run anything.
//
// The daemon fills the address's line with 1. A symbol opens at its own line only when the page can
// tell, from the address alone, where that 1 is: right after the file's path, as `:1` (Visual Studio
// Code's form) or as a `line=1` parameter (JetBrains' form), with no other digit after the path.
// Any other address opens the file at line 1, and the panel says so.

/** The bytes of a path the daemon writes as they are; every other byte is written `%XX`. */
const PLAIN_PATH_BYTE = /^[A-Za-z0-9\-._~/:]$/;

/** The part after the path that holds the line: `:1`, or `?line=1` / `&line=1`, then no digit. */
const LINE_AFTER_PATH = /^(:|[?&]line=)1([&#][^0-9]*)?$/;

/** `path` written as the daemon writes it into the address: UTF-8 bytes, `%XX` but for plain ones. */
export function encodePath(path: string): string {
  let encoded = '';
  for (const byte of new TextEncoder().encode(path)) {
    const char = String.fromCharCode(byte);
    encoded +=
      byte < 0x80 && PLAIN_PATH_BYTE.test(char)
        ? char
        : `%${byte.toString(16).toUpperCase().padStart(2, '0')}`;
  }
  return encoded;
}

/** Where an editor link opens a symbol: at its line, or, when that cannot be told, at line 1. */
export interface LineLink {
  readonly href: string;
  readonly isAtLine: boolean;
}

/**
 * The address that opens file `path` at `line`, from the daemon's address `url` for it (which
 * opens it at line 1); null when there is no link to follow.
 */
export function editorHrefAt(url: string | null, path: string, line: number): LineLink | null {
  const atFirst = editorHref(url);
  if (url === null || atFirst === null) return null;
  const marker = `/${encodePath(path)}`;
  const at = url.lastIndexOf(marker);
  if (at < 0) return { href: atFirst, isAtLine: false };
  const after = url.slice(at + marker.length);
  const match = LINE_AFTER_PATH.exec(after);
  if (match === null) return { href: atFirst, isAtLine: false };
  const lead = match[1] ?? '';
  const rest = match[2] ?? '';
  const href = editorHref(`${url.slice(0, at + marker.length)}${lead}${line}${rest}`);
  return href === null ? { href: atFirst, isAtLine: false } : { href, isAtLine: true };
}

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
