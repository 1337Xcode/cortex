// Tests for the editor link check: an editor's own scheme is followed, everything the page must
// never follow is refused, whatever its case or the whitespace a URL parser strips.
import { describe, expect, it } from 'vitest';
import { editorHref } from './editorLink';

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
  ])('refuses %s', (url) => {
    expect(editorHref(url)).toBeNull();
  });
});
