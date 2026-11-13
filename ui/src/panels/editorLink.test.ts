// Tests for the editor link check: an editor's own scheme is followed, everything the page must
// never follow is refused, whatever its case or the whitespace a URL parser strips.
import { describe, expect, it } from 'vitest';
import { editorHref, editorHrefAt, encodePath } from './editorLink';

describe('editorHrefAt', () => {
  it("puts a symbol's line where Visual Studio Code's and JetBrains' addresses hold it", () => {
    expect(
      editorHrefAt('vscode://file/C:/repo/src/orders/book.rs:1', 'src/orders/book.rs', 40),
    ).toEqual({
      href: 'vscode://file/C:/repo/src/orders/book.rs:40',
      isAtLine: true,
    });
    expect(editorHrefAt('idea://open?file=/home/me/repo/src/a.rs&line=1', 'src/a.rs', 7)).toEqual({
      href: 'idea://open?file=/home/me/repo/src/a.rs&line=7',
      isAtLine: true,
    });
  });

  it('finds the path as the daemon writes it, a space and a non-ASCII letter written %XX', () => {
    // "é" is the bytes C3 A9 in UTF-8; a space is 20.
    expect(encodePath('src/my file/é.rs')).toBe('src/my%20file/%C3%A9.rs');
    expect(
      editorHrefAt('vscode://file/C:/repo/src/my%20file/%C3%A9.rs:1', 'src/my file/é.rs', 3)?.href,
    ).toBe('vscode://file/C:/repo/src/my%20file/%C3%A9.rs:3');
  });

  it('opens at line 1, and says so, when it cannot tell where the line goes', () => {
    // The line before the path; a column after the line; no line at all; a version number after.
    for (const url of [
      'ed://open?line=1&file=/repo/a.rs',
      'vscode://file/repo/a.rs:1:1',
      'zed://file/repo/a.rs',
      'ed://open?file=/repo/a.rs&line=1&v=2',
    ]) {
      expect(editorHrefAt(url, 'a.rs', 9)).toEqual({ href: url, isAtLine: false });
    }
  });

  it('shows nothing when the link is off or may not be followed', () => {
    expect(editorHrefAt(null, 'a.rs', 9)).toBeNull();
    expect(editorHrefAt('javascript:alert(1)//a.rs:1', 'a.rs', 9)).toBeNull();
  });
});

describe('editorHref', () => {
  it('follows an editor scheme as the daemon wrote it', () => {
    expect(editorHref('vscode://file/C:/repo/src/orders/book.rs:1')).toBe(
      'vscode://file/C:/repo/src/orders/book.rs:1',
    );
    expect(editorHref('cursor://file/home/me/repo/a.rs:12')).toBe(
      'cursor://file/home/me/repo/a.rs:12',
    );
  });

  it('shows nothing when the link is turned off or is not a URL', () => {
    expect(editorHref(null)).toBeNull();
    expect(editorHref('')).toBeNull();
    expect(editorHref('src/orders/book.rs')).toBeNull();
  });

  it.each([
    'http://127.0.0.1:47613/x',
    'https://example.com/',
    'javascript:alert(1)',
    'JavaScript:alert(1)',
    ' javascript:alert(1)',
    'java\tscript:alert(1)',
    'data:text/html,<b>x</b>',
    'file:///C:/repo/a.rs',
    'blob:http://127.0.0.1:47613/0b7a',
    'BLOB:x',
    'vbscript:msgbox(1)',
    'VBScript:msgbox(1)',
  ])('refuses %s', (url) => {
    expect(editorHref(url)).toBeNull();
  });
});
