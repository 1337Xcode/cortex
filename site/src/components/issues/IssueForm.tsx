import { useCallback, useEffect, useRef, useState } from 'react';
import DOMPurify from 'isomorphic-dompurify';
import { z } from 'zod';

const issueSchema = z.object({
  type: z.enum(['bug', 'feature', 'question', 'security']),
  title: z.string().min(5).max(120),
  description: z.string().min(20).max(8000),
  version: z.string().min(1).max(30),
});

const OAUTH_ERRORS: Record<string, string> = {
  oauth_state: 'Sign-in expired or was interrupted. Please try again.',
  oauth_token: 'GitHub authorization failed. Please try again.',
};

interface IssueFormProps {
  apiUrl?: string;
  turnstileKey?: string;
}

const SESSION_KEY = 'cortex_issues_session';
const TURNSTILE_SCRIPT_ID = 'cf-turnstile-api';

declare global {
  interface Window {
    turnstile?: {
      render: (el: HTMLElement, opts: { sitekey: string; callback: (token: string) => void }) => string;
      reset: (id: string) => void;
    };
  }
}

function stripQueryParam(name: string) {
  const params = new URLSearchParams(window.location.search);
  if (!params.has(name)) return null;
  const value = params.get(name);
  params.delete(name);
  const clean = `${window.location.pathname}${params.toString() ? `?${params}` : ''}`;
  window.history.replaceState({}, '', clean);
  return value;
}

export default function IssueForm({
  apiUrl: apiUrlProp,
  turnstileKey: turnstileKeyProp,
}: IssueFormProps = {}) {
  const API_URL = apiUrlProp ?? import.meta.env.PUBLIC_ISSUES_API_URL ?? '';
  const TURNSTILE_KEY =
    turnstileKeyProp ??
    import.meta.env.PUBLIC_TURNSTILE_SITE_KEY ??
    (import.meta.env as { TURNSTILE_SITE?: string }).TURNSTILE_SITE ??
    '';

  const [type, setType] = useState<'bug' | 'feature' | 'question' | 'security'>('bug');
  const [version, setVersion] = useState<string>('latest');
  const [community, setCommunity] = useState<{
    links?: { discussions: string; issues: string; releases: string };
    recent?: { number: number; title: string; url: string; labels: string[]; reactions: number }[];
  } | null>(null);
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [login, setLogin] = useState<string | null>(null);
  const [session, setSession] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [cooldown, setCooldown] = useState(false);
  const [turnstileToken, setTurnstileToken] = useState('');
  const turnstileRef = useRef<HTMLDivElement>(null);
  const widgetId = useRef<string | null>(null);
  const [website, setWebsite] = useState('');
  const [company, setCompany] = useState('');
  const [trapUrl, setTrapUrl] = useState('');
  const [fax, setFax] = useState('');
  const [subject, setSubject] = useState('');
  const [formToken, setFormToken] = useState('');

  useEffect(() => {
    const oauthErr = stripQueryParam('error');
    if (oauthErr && OAUTH_ERRORS[oauthErr]) {
      setError(OAUTH_ERRORS[oauthErr]);
    }

    const sessionParam = stripQueryParam('session');
    if (sessionParam) {
      sessionStorage.setItem(SESSION_KEY, sessionParam);
      setSession(sessionParam);
      return;
    }

    const stored = sessionStorage.getItem(SESSION_KEY);
    if (stored) setSession(stored);
  }, []);

  useEffect(() => {
    (window as any).__formLoadTime = Date.now();
    if (!API_URL) return;
    fetch(`${API_URL}/api/form-bootstrap`)
      .then((r) => r.json())
      .then((data: { formToken?: string }) => {
        if (data.formToken) setFormToken(data.formToken);
      })
      .catch(() => {});
    fetch(`${API_URL}/api/community`)
      .then((r) => r.json())
      .then((data) => setCommunity(data))
      .catch(() => {});
  }, [API_URL]);

  useEffect(() => {
    if (!API_URL || !session) return;
    fetch(`${API_URL}/api/me`, {
      headers: { Authorization: `Bearer ${session}` },
    })
      .then((r) => r.json())
      .then((data: { authenticated?: boolean; login?: string }) => {
        if (data.authenticated && data.login) setLogin(data.login);
        else {
          sessionStorage.removeItem(SESSION_KEY);
          setSession(null);
          setLogin(null);
        }
      })
      .catch(() => setError('Could not verify GitHub session.'));
  }, [API_URL, session]);

  const renderTurnstile = useCallback(() => {
    if (!TURNSTILE_KEY || !login || !turnstileRef.current || widgetId.current) return;
    if (!window.turnstile) return;
    widgetId.current = window.turnstile.render(turnstileRef.current, {
      sitekey: TURNSTILE_KEY,
      callback: (token: string) => setTurnstileToken(token),
    });
  }, [TURNSTILE_KEY, login]);

  useEffect(() => {
    if (!login || !TURNSTILE_KEY) return;

    if (window.turnstile) {
      renderTurnstile();
      return;
    }

    let script = document.getElementById(TURNSTILE_SCRIPT_ID) as HTMLScriptElement | null;
    if (!script) {
      script = document.createElement('script');
      script.id = TURNSTILE_SCRIPT_ID;
      script.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';
      script.async = true;
      document.body.appendChild(script);
    }

    const onReady = () => renderTurnstile();
    script.addEventListener('load', onReady);
    if (window.turnstile) onReady();

    return () => {
      script?.removeEventListener('load', onReady);
    };
  }, [login, TURNSTILE_KEY, renderTurnstile]);

  function signIn() {
    if (!API_URL) {
      setError('Issues API is not configured. Set PUBLIC_ISSUES_API_URL.');
      return;
    }
    setError(null);
    const returnTo = `${window.location.origin}${window.location.pathname}`;
    window.location.href = `${API_URL}/auth/github?return_to=${encodeURIComponent(returnTo)}`;
  }

  function signOut() {
    sessionStorage.removeItem(SESSION_KEY);
    setSession(null);
    setLogin(null);
    setTurnstileToken('');
    widgetId.current = null;
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setSuccess(null);

    if (website || company || trapUrl || fax || subject) return;

    // Anti-bot: form must be open for at least 3 seconds
    const elapsed = Date.now() - (window as any).__formLoadTime;
    if (elapsed < 3000) {
      return; // Silent reject - bots submit too fast
    }

    const parsed = issueSchema.safeParse({ type, title, description, version });
    if (!parsed.success) {
      setError(parsed.error.errors[0]?.message || 'Invalid form data.');
      return;
    }

    if (!session || !login) {
      setError('Sign in with GitHub first.');
      return;
    }

    if (!API_URL) {
      setError('Issues API is not configured.');
      return;
    }

    if (TURNSTILE_KEY && !turnstileToken) {
      setError('Complete the verification challenge.');
      return;
    }

    if (!formToken) {
      // Allow submission without formToken after 5s (API might be slow)
      const elapsed = Date.now() - (window as any).__formLoadTime;
      if (elapsed < 5000) {
        setError('Form is still loading. Wait a moment and try again.');
        return;
      }
    }

    setLoading(true);
    try {
      const res = await fetch(`${API_URL}/api/issues`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${session}`,
        },
        body: JSON.stringify({
          type: parsed.data.type,
          version: parsed.data.version,
          title: DOMPurify.sanitize(parsed.data.title, { ALLOWED_TAGS: [] }),
          description: DOMPurify.sanitize(parsed.data.description, { ALLOWED_TAGS: [] }),
          turnstileToken: turnstileToken || 'disabled',
          formToken,
          website,
          company,
          url: trapUrl,
          fax,
          subject,
        }),
      });
      const data = (await res.json()) as { error?: string; url?: string };
      if (!res.ok) {
        setError(data.error || 'Failed to create issue.');
        return;
      }
      setSuccess(data.url || 'Issue created.');
      setCooldown(true);
      setTimeout(() => setCooldown(false), 10000);
      setTitle('');
      setDescription('');
      if (widgetId.current && window.turnstile) window.turnstile.reset(widgetId.current);
      setTurnstileToken('');
    } catch {
      setError('Network error. Try again.');
    } finally {
      setLoading(false);
    }
  }

  if (!API_URL) {
    return (
      <p className="text-sm text-muted-foreground rounded-lg border border-border bg-muted/30 p-4 leading-relaxed">
        The issues API URL is not in this build. Add repository secrets{' '}
        <code className="font-mono text-xs">PUBLIC_ISSUES_API_URL</code> and optionally{' '}
        <code className="font-mono text-xs">PUBLIC_TURNSTILE_SITE_KEY</code>, deploy the Cloudflare Worker (
        <code className="font-mono text-xs">site/workers/issues-api</code>), then re-run the{' '}
        <strong>Deploy Site</strong> workflow.
      </p>
    );
  }

  return (
    <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_280px] lg:gap-10">
      <div className="space-y-6 min-w-0">
        {!login ? (
          <div className="rounded-xl border border-border bg-card p-6 sm:p-8 text-center space-y-4">
            <p className="text-sm text-muted-foreground max-w-md mx-auto leading-relaxed">
              Sign in with GitHub to open an issue on the Cortex repository. Your session stays in this browser tab only.
            </p>
            <button
              type="button"
              onClick={signIn}
              className="inline-flex items-center justify-center rounded-md bg-primary text-primary-foreground px-5 py-2.5 text-sm font-medium hover:opacity-90 transition-opacity"
            >
              Sign in with GitHub
            </button>
          </div>
        ) : (
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border bg-muted/20 px-4 py-3">
            <p className="text-sm">
              Signed in as <span className="font-medium">@{login}</span>
            </p>
            <button
              type="button"
              onClick={signOut}
              className="text-sm text-muted-foreground hover:text-foreground underline-offset-2 hover:underline"
            >
              Sign out
            </button>
          </div>
        )}

        {error && (
          <p className="text-sm text-destructive rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3" role="alert">
            {error}
          </p>
        )}

        <form onSubmit={onSubmit} className="space-y-5 rounded-xl border border-border bg-card p-5 sm:p-6">
          <div className="sr-only" aria-hidden>
            <label htmlFor="issue-website">Website</label>
            <input
              id="issue-website"
              type="text"
              name="website"
              value={website}
              onChange={(e) => setWebsite(e.target.value)}
              tabIndex={-1}
              autoComplete="off"
            />
            <label htmlFor="issue-company">Company</label>
            <input
              id="issue-company"
              type="text"
              name="company"
              value={company}
              onChange={(e) => setCompany(e.target.value)}
              tabIndex={-1}
              autoComplete="off"
            />
            <label htmlFor="issue-url">URL</label>
            <input
              id="issue-url"
              type="text"
              name="url"
              value={trapUrl}
              onChange={(e) => setTrapUrl(e.target.value)}
              tabIndex={-1}
              autoComplete="off"
            />
            <label htmlFor="issue-fax">Fax</label>
            <input
              id="issue-fax"
              type="text"
              name="fax"
              value={fax}
              onChange={(e) => setFax(e.target.value)}
              tabIndex={-1}
              autoComplete="off"
            />
            <label htmlFor="issue-subject">Subject</label>
            <input
              id="issue-subject"
              type="text"
              name="subject"
              value={subject}
              onChange={(e) => setSubject(e.target.value)}
              tabIndex={-1}
              autoComplete="off"
            />
          </div>

          <div className="grid gap-5 sm:grid-cols-2">
            <div>
              <label htmlFor="issue-type" className="block text-sm font-medium mb-2">
                Type
              </label>
              <select
                id="issue-type"
                value={type}
                onChange={(e) => setType(e.target.value as typeof type)}
                className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-1"
                disabled={!login}
              >
                <option value="bug">Bug report</option>
                <option value="feature">Feature request</option>
                <option value="question">Question</option>
                <option value="security">Security concern</option>
              </select>
            </div>

            <div>
              <label htmlFor="issue-version" className="block text-sm font-medium mb-2">
                Cortex version
              </label>
              <div className="flex gap-2">
                <select
                  id="issue-version"
                  value={version}
                  onChange={(e) => setVersion(e.target.value)}
                  className="flex-1 rounded-lg border border-input bg-background px-3 py-2 text-sm appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-1"
                  disabled={!login}
                >
                  <option value="latest">Latest release</option>
                  <option value="1.0.0">1.0.0</option>
                  <option value="main">main branch</option>
                  <option value="other">Other (specify below)</option>
                </select>
              </div>
              {version === 'other' && (
                <input
                  type="text"
                  placeholder="e.g. 1.0.0"
                  className="mt-2 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-1"
                  onChange={(e) => setVersion(e.target.value as any)}
                  disabled={!login}
                />
              )}
            </div>
          </div>

          <div>
            <label htmlFor="issue-title" className="block text-sm font-medium mb-2">
              Title
            </label>
            <input
              id="issue-title"
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              maxLength={120}
              required
              disabled={!login}
              className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm placeholder:text-muted-foreground"
              placeholder="Short summary"
            />
          </div>

          <div>
            <label htmlFor="issue-body" className="block text-sm font-medium mb-2">
              Description
            </label>
            <textarea
              id="issue-body"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={8}
              maxLength={8000}
              required
              disabled={!login}
              className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm placeholder:text-muted-foreground resize-y min-h-[160px]"
              placeholder="Steps to reproduce, expected behavior, environment, etc."
            />
          </div>

          {login && TURNSTILE_KEY ? <div ref={turnstileRef} className="min-h-[65px]" /> : null}

          {success && (
            <p className="text-sm rounded-lg border border-border bg-muted/30 px-4 py-3">
              Issue created:{' '}
              <a href={success} className="underline font-medium" target="_blank" rel="noopener noreferrer">
                View on GitHub
              </a>
            </p>
          )}

          <button
            type="submit"
            disabled={!login || loading || cooldown}
            className="inline-flex w-full sm:w-auto items-center justify-center rounded-md bg-primary text-primary-foreground px-5 py-2.5 text-sm font-medium hover:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed transition-opacity"
          >
            {loading ? 'Submitting…' : cooldown ? 'Please wait…' : 'Submit issue'}
          </button>
        </form>
      </div>

      <aside className="space-y-4 lg:sticky lg:top-24 lg:self-start min-w-0">
        <div className="rounded-xl border border-border bg-card p-4">
          <h2 className="text-sm font-semibold mb-2">Community</h2>
          <p className="text-xs text-muted-foreground mb-3 leading-relaxed">
            Vote with GitHub reactions. Use Discussions for polls and feature brainstorming.
          </p>
          <div className="flex flex-col gap-2 text-sm">
            {community?.links?.discussions && (
              <a
                href={community.links.discussions}
                target="_blank"
                rel="noopener noreferrer"
                className="underline-offset-2 hover:underline"
              >
                Discussions and polls
              </a>
            )}
            {community?.links?.releases && (
              <a
                href={community.links.releases}
                target="_blank"
                rel="noopener noreferrer"
                className="underline-offset-2 hover:underline"
              >
                Releases
              </a>
            )}
          </div>
        </div>
        {community?.recent && community.recent.length > 0 && (
          <div className="rounded-xl border border-border bg-card p-4">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-3">
              Open issues
            </h3>
            <ul className="space-y-2.5">
              {community.recent.slice(0, 5).map((item) => (
                <li key={item.number}>
                  <a
                    href={item.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-sm hover:underline line-clamp-2 leading-snug"
                  >
                    #{item.number} {item.title}
                  </a>
                  {item.reactions > 0 && (
                    <span className="text-[10px] text-muted-foreground ml-1">+{item.reactions}</span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}
      </aside>
    </div>
  );
}
