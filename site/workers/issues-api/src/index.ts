import { Octokit } from '@octokit/rest';

export interface Env {
  GITHUB_CLIENT_ID: string;
  GITHUB_CLIENT_SECRET: string;
  GITHUB_REPO: string;
  /** PAT with `issues:write` only. Never exposed to clients. */
  GITHUB_ISSUES_TOKEN: string;
  /** Optional PAT with `public_repo` read for community feed. */
  GITHUB_READ_TOKEN?: string;
  SESSION_SECRET: string;
  TURNSTILE_SECRET: string;
  ALLOWED_ORIGINS: string;
  SITE_URL?: string;
}

interface SessionPayload {
  login: string;
  exp: number;
}

const ISSUE_TYPES: Record<string, { label: string; githubLabel: string }> = {
  bug: { label: 'Bug', githubLabel: 'bug' },
  feature: { label: 'Feature', githubLabel: 'enhancement' },
  question: { label: 'Question', githubLabel: 'question' },
  security: { label: 'Security', githubLabel: 'security' },
};

const VERSIONS = ['latest', '0.1.x', 'main', 'other'] as const;

const RATE_IP_WINDOW_MS = 60_000;
const RATE_IP_MAX = 30;
const RATE_USER_WINDOW_MS = 60_000;
const RATE_USER_MAX = 5;
const RATE_ISSUE_WINDOW_MS = 3_600_000;
const RATE_ISSUE_MAX = 3;
const MAX_BODY_BYTES = 12_000;

const ipRateMap = new Map<string, { count: number; reset: number }>();
const userRateMap = new Map<string, { count: number; reset: number }>();
const issueRateMap = new Map<string, { count: number; reset: number }>();
const recentHashes = new Map<string, number>();

function securityHeaders(): HeadersInit {
  return {
    'X-Content-Type-Options': 'nosniff',
    'X-Frame-Options': 'DENY',
    'Referrer-Policy': 'strict-origin-when-cross-origin',
    'Permissions-Policy': 'interest-cohort=()',
    'Content-Security-Policy': "default-src 'none'; frame-ancestors 'none'",
  };
}

function corsHeaders(origin: string | null, env: Env): HeadersInit {
  const allowed = env.ALLOWED_ORIGINS.split(',').map((o) => o.trim()).filter(Boolean);
  const headers: Record<string, string> = {
    'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
    'Access-Control-Allow-Headers': 'Content-Type, Authorization',
    'Access-Control-Max-Age': '86400',
    ...securityHeaders() as Record<string, string>,
  };
  if (origin && allowed.includes(origin)) {
    headers['Access-Control-Allow-Origin'] = origin;
    headers['Vary'] = 'Origin';
  }
  return headers;
}

function json(data: unknown, status = 200, extra: HeadersInit = {}): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'Content-Type': 'application/json; charset=utf-8', ...extra },
  });
}

function redirect(url: string, extra: HeadersInit = {}): Response {
  return new Response(null, { status: 302, headers: { Location: url, ...extra } });
}

async function hmacSign(secret: string, payload: string): Promise<string> {
  const key = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const sig = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(payload));
  return btoa(String.fromCharCode(...new Uint8Array(sig)))
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/, '');
}

function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let out = 0;
  for (let i = 0; i < a.length; i++) out |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return out === 0;
}

async function createSession(env: Env, payload: SessionPayload): Promise<string> {
  const body = btoa(JSON.stringify(payload));
  const sig = await hmacSign(env.SESSION_SECRET, body);
  return `${body}.${sig}`;
}

interface OAuthStatePayload {
  n: string;
  r: string;
  exp: number;
}

function base64UrlEncode(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/, '');
}

function base64UrlDecodeToString(input: string): string {
  const padded = input.replace(/-/g, '+').replace(/_/g, '/');
  const pad = padded.length % 4 === 0 ? '' : '='.repeat(4 - (padded.length % 4));
  return atob(padded + pad);
}

async function encodeOAuthState(env: Env, returnTo: string): Promise<string> {
  const payload: OAuthStatePayload = {
    n: crypto.randomUUID(),
    r: returnTo,
    exp: Date.now() + 600_000,
  };
  const body = base64UrlEncode(new TextEncoder().encode(JSON.stringify(payload)));
  const sig = await hmacSign(env.SESSION_SECRET, body);
  return `${body}.${sig}`;
}

async function decodeOAuthState(env: Env, state: string): Promise<OAuthStatePayload | null> {
  const [body, sig] = state.split('.');
  if (!body || !sig) return null;
  const expected = await hmacSign(env.SESSION_SECRET, body);
  if (!timingSafeEqual(sig, expected)) return null;
  try {
    const payload = JSON.parse(base64UrlDecodeToString(body)) as OAuthStatePayload;
    if (!payload.r || payload.exp < Date.now()) return null;
    if (!isValidReturnUrl(payload.r, env)) return null;
    return payload;
  } catch {
    return null;
  }
}

interface FormTokenPayload {
  n: string;
  nb: number;
  exp: number;
}

const HONEYPOT_FIELDS = ['website', 'company', 'url', 'fax', 'subject'] as const;

async function createFormToken(env: Env): Promise<{ formToken: string; notBefore: number }> {
  const payload: FormTokenPayload = {
    n: crypto.randomUUID(),
    nb: Date.now() + 2_500,
    exp: Date.now() + 900_000,
  };
  const body = base64UrlEncode(new TextEncoder().encode(JSON.stringify(payload)));
  const sig = await hmacSign(env.SESSION_SECRET, body);
  return { formToken: `${body}.${sig}`, notBefore: payload.nb };
}

async function validateFormToken(env: Env, token: string): Promise<boolean> {
  const [body, sig] = token.split('.');
  if (!body || !sig) return false;
  const expected = await hmacSign(env.SESSION_SECRET, body);
  if (!timingSafeEqual(sig, expected)) return false;
  try {
    const payload = JSON.parse(base64UrlDecodeToString(body)) as FormTokenPayload;
    if (!payload.n || payload.exp < Date.now()) return false;
    if (payload.nb > Date.now()) return false;
    return true;
  } catch {
    return false;
  }
}

function requestFromAllowedSite(request: Request, env: Env): boolean {
  const origin = request.headers.get('Origin');
  if (origin && isValidReturnUrl(origin, env)) return true;
  const referer = request.headers.get('Referer');
  if (!referer) return false;
  try {
    return isValidReturnUrl(new URL(referer).origin, env);
  } catch {
    return false;
  }
}

function honeypotsTriggered(body: Record<string, unknown>): boolean {
  return HONEYPOT_FIELDS.some((field) => {
    const value = body[field];
    return typeof value === 'string' && value.trim().length > 0;
  });
}

function containsSuspiciousMarkup(text: string): boolean {
  return /<script|javascript:|on\w+\s*=|data:text\/html|<iframe/i.test(text);
}

async function parseSession(env: Env, token: string | null): Promise<SessionPayload | null> {
  if (!token) return null;
  const [body, sig] = token.split('.');
  if (!body || !sig) return null;
  const expected = await hmacSign(env.SESSION_SECRET, body);
  if (!timingSafeEqual(sig, expected)) return null;
  try {
    const data = JSON.parse(atob(body)) as SessionPayload;
    if (data.exp < Date.now()) return null;
    if (!data.login) return null;
    return data;
  } catch {
    return null;
  }
}

function getCookie(request: Request, name: string): string | null {
  const header = request.headers.get('Cookie') || '';
  const match = header.match(new RegExp(`(?:^|;\\s*)${name}=([^;]*)`));
  return match ? decodeURIComponent(match[1]) : null;
}

function checkRate(
  map: Map<string, { count: number; reset: number }>,
  key: string,
  max: number,
  windowMs: number,
): boolean {
  const now = Date.now();
  const entry = map.get(key);
  if (!entry || now > entry.reset) {
    map.set(key, { count: 1, reset: now + windowMs });
    return true;
  }
  if (entry.count >= max) return false;
  entry.count += 1;
  return true;
}

function sanitizeText(input: string, maxLen: number): string {
  return input
    .replace(/[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]/g, '')
    .replace(/```[\s\S]*?```/g, '[code removed]')
    .trim()
    .slice(0, maxLen);
}

async function verifyTurnstile(env: Env, token: string, ip: string): Promise<boolean> {
  if (!env.TURNSTILE_SECRET) return false;
  if (!token || token.length > 2048) return false;
  const res = await fetch('https://challenges.cloudflare.com/turnstile/v0/siteverify', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      secret: env.TURNSTILE_SECRET,
      response: token,
      remoteip: ip,
    }),
  });
  const data = (await res.json()) as { success?: boolean };
  return Boolean(data.success);
}

function siteReturnUrl(env: Env, path: string): string {
  const base = env.SITE_URL || 'https://1337xcode.github.io/cortex';
  return `${base.replace(/\/$/, '')}${path}`;
}

function isValidReturnUrl(url: string, env: Env): boolean {
  try {
    const parsed = new URL(url);
    const allowed = env.ALLOWED_ORIGINS.split(',').map((o) => o.trim());
    return allowed.some((origin) => {
      const o = new URL(origin);
      return parsed.origin === o.origin;
    });
  } catch {
    return false;
  }
}

async function sessionFromRequest(req: Request, env: Env): Promise<SessionPayload | null> {
  const auth = req.headers.get('Authorization');
  if (auth?.startsWith('Bearer ')) {
    return parseSession(env, auth.slice(7).trim());
  }
  return parseSession(env, getCookie(req, 'cortex_session'));
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const origin = request.headers.get('Origin');
    const cors = corsHeaders(origin, env);
    const ip = request.headers.get('CF-Connecting-IP') || 'unknown';

    if (request.method === 'OPTIONS') {
      return new Response(null, { status: 204, headers: cors });
    }

    if (!checkRate(ipRateMap, ip, RATE_IP_MAX, RATE_IP_WINDOW_MS)) {
      return json({ error: 'Too many requests. Try again later.' }, 429, cors);
    }

    const contentLength = Number(request.headers.get('Content-Length') || 0);
    if (contentLength > MAX_BODY_BYTES) {
      return json({ error: 'Request too large.' }, 413, cors);
    }

    try {
      if (url.pathname === '/auth/github' && request.method === 'GET') {
        let returnTo = url.searchParams.get('return_to') || siteReturnUrl(env, '/issues/');
        if (!isValidReturnUrl(returnTo, env)) {
          returnTo = siteReturnUrl(env, '/issues/');
        }
        const state = await encodeOAuthState(env, returnTo);
        const authorize = new URL('https://github.com/login/oauth/authorize');
        authorize.searchParams.set('client_id', env.GITHUB_CLIENT_ID);
        authorize.searchParams.set('redirect_uri', `${url.origin}/auth/callback`);
        authorize.searchParams.set('scope', 'read:user');
        authorize.searchParams.set('state', state);
        return redirect(authorize.toString(), cors);
      }

      if (url.pathname === '/auth/callback' && request.method === 'GET') {
        const code = url.searchParams.get('code');
        const state = url.searchParams.get('state') || '';
        const payload = await decodeOAuthState(env, state);
        const returnTo = payload?.r || siteReturnUrl(env, '/issues/');

        if (!code || !payload) {
          return redirect(`${returnTo}?error=oauth_state`);
        }

        const tokenRes = await fetch('https://github.com/login/oauth/access_token', {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            Accept: 'application/json',
            'User-Agent': 'cortex-issues-worker',
          },
          body: JSON.stringify({
            client_id: env.GITHUB_CLIENT_ID,
            client_secret: env.GITHUB_CLIENT_SECRET,
            code,
          }),
        });
        const tokenData = (await tokenRes.json()) as { access_token?: string };
        if (!tokenData.access_token) {
          return redirect(`${returnTo}?error=oauth_token`);
        }

        const octokit = new Octokit({ auth: tokenData.access_token });
        const { data: user } = await octokit.users.getAuthenticated();
        const session = await createSession(env, {
          login: user.login,
          exp: Date.now() + 60 * 60 * 1000,
        });
        const sep = returnTo.includes('?') ? '&' : '?';
        const target = `${returnTo}${sep}session=${encodeURIComponent(session)}`;
        return redirect(target, cors);
      }

      if (url.pathname === '/api/me' && request.method === 'GET') {
        const session = await sessionFromRequest(request, env);
        if (!session) return json({ authenticated: false }, 200, cors);
        return json({ authenticated: true, login: session.login }, 200, cors);
      }

      if (url.pathname === '/api/form-bootstrap' && request.method === 'GET') {
        if (!requestFromAllowedSite(request, env)) {
          return json({ error: 'Forbidden.' }, 403, cors);
        }
        const bootstrap = await createFormToken(env);
        return json(bootstrap, 200, cors);
      }

      if (url.pathname === '/api/community' && request.method === 'GET') {
        const [owner, repo] = env.GITHUB_REPO.split('/');
        const links = {
          discussions: `https://github.com/${owner}/${repo}/discussions`,
          issues: `https://github.com/${owner}/${repo}/issues`,
          releases: `https://github.com/${owner}/${repo}/releases`,
        };
        let recent: { number: number; title: string; url: string; labels: string[]; reactions: number }[] = [];
        if (env.GITHUB_READ_TOKEN) {
          const octokit = new Octokit({ auth: env.GITHUB_READ_TOKEN });
          const { data } = await octokit.issues.listForRepo({
            owner,
            repo,
            state: 'open',
            per_page: 8,
            sort: 'updated',
          });
          recent = data
            .filter((i) => !i.pull_request)
            .map((i) => ({
              number: i.number,
              title: i.title.slice(0, 120),
              url: i.html_url,
              labels: (i.labels || []).map((l) => (typeof l === 'string' ? l : l.name || '')).filter(Boolean),
              reactions: i.reactions?.['+1'] || 0,
            }));
        }
        return json({ links, recent, versions: VERSIONS }, 200, cors);
      }

      if (url.pathname === '/auth/logout' && request.method === 'POST') {
        return json({ ok: true }, 200, cors);
      }

      if (url.pathname === '/api/issues' && request.method === 'POST') {
        if (!requestFromAllowedSite(request, env)) {
          return json({ error: 'Forbidden.' }, 403, cors);
        }

        const session = await sessionFromRequest(request, env);
        if (!session) {
          return json({ error: 'Sign in with GitHub to submit feedback.' }, 401, cors);
        }

        if (!checkRate(userRateMap, session.login, RATE_USER_MAX, RATE_USER_WINDOW_MS)) {
          return json({ error: 'Slow down. Try again in a minute.' }, 429, cors);
        }
        if (!checkRate(issueRateMap, session.login, RATE_ISSUE_MAX, RATE_ISSUE_WINDOW_MS)) {
          return json({ error: 'Issue limit reached. Try again later.' }, 429, cors);
        }

        const contentType = request.headers.get('Content-Type') || '';
        if (!contentType.includes('application/json')) {
          return json({ error: 'Invalid content type.' }, 415, cors);
        }

        const body = (await request.json()) as {
          type?: string;
          title?: string;
          description?: string;
          version?: string;
          turnstileToken?: string;
          website?: string;
          company?: string;
          url?: string;
          fax?: string;
          subject?: string;
          formToken?: string;
        };

        if (honeypotsTriggered(body)) {
          return json({ error: 'Rejected.' }, 400, cors);
        }

        if (!body.formToken || !(await validateFormToken(env, body.formToken))) {
          return json({ error: 'Form expired. Refresh the page and try again.' }, 400, cors);
        }

        const type = body.type && ISSUE_TYPES[body.type] ? body.type : 'bug';
        const title = sanitizeText(String(body.title || ''), 120);
        const description = sanitizeText(String(body.description || ''), 8000);
        const version = VERSIONS.includes((body.version || 'latest') as (typeof VERSIONS)[number])
          ? body.version
          : 'other';

        if (title.length < 5) {
          return json({ error: 'Title must be at least 5 characters.' }, 400, cors);
        }
        if (description.length < 20) {
          return json({ error: 'Description must be at least 20 characters.' }, 400, cors);
        }

        if (containsSuspiciousMarkup(title) || containsSuspiciousMarkup(description)) {
          return json({ error: 'Markup and scripts are not allowed in issue text.' }, 400, cors);
        }

        if (env.TURNSTILE_SECRET) {
          if (!body.turnstileToken || body.turnstileToken === 'disabled') {
            return json({ error: 'Complete the verification challenge and retry.' }, 400, cors);
          }
          if (!(await verifyTurnstile(env, body.turnstileToken, ip))) {
            return json({ error: 'Verification failed. Complete the challenge and retry.' }, 400, cors);
          }
        }

        const hashKey = `${session.login}:${title.toLowerCase()}`;
        const now = Date.now();
        const last = recentHashes.get(hashKey);
        if (last && now - last < RATE_ISSUE_WINDOW_MS) {
          return json({ error: 'Duplicate submission. Edit your existing issue on GitHub.' }, 409, cors);
        }

        const typeMeta = ISSUE_TYPES[type];
        const issueBody = [
          `**Type:** ${typeMeta.label}`,
          `**Version:** ${version}`,
          `**Reported by:** @${session.login} (cortex site)`,
          '',
          '---',
          '',
          description,
          '',
          '---',
          '_React with :+1: on GitHub to vote. Discussions are open for polls and ideas._',
        ].join('\n');

        const [owner, repo] = env.GITHUB_REPO.split('/');
        if (!env.GITHUB_ISSUES_TOKEN) {
          return json({ error: 'Issue submission is not configured.' }, 503, cors);
        }
        const octokit = new Octokit({ auth: env.GITHUB_ISSUES_TOKEN });
        const { data: issue } = await octokit.issues.create({
          owner,
          repo,
          title: `[${typeMeta.label}] ${title}`,
          body: issueBody,
          labels: [typeMeta.githubLabel, `version:${version}`],
        });

        recentHashes.set(hashKey, now);

        return json(
          {
            ok: true,
            url: issue.html_url,
            number: issue.number,
            reactUrl: `${issue.html_url}#reactions`,
          },
          201,
          cors,
        );
      }

      return json({ error: 'Not found' }, 404, cors);
    } catch (err) {
      console.error(err);
      return json({ error: 'Something went wrong. Try again.' }, 500, cors);
    }
  },
};
