# Cortex Issues API (Cloudflare Worker)

Secure GitHub OAuth + issue creation for the static Cortex site.

## Do not use the dashboard folder uploader

Cloudflare’s **“upload files” / drag-folder** flow only accepts plain JavaScript. This project is **TypeScript** and depends on `@octokit/rest`, so the dashboard shows:

> This uploader does not yet support projects that require a build process…

That is expected. Deploy with **Wrangler** (CLI), which bundles TypeScript and npm dependencies for you.

## Deploy (recommended)

### 1. Prerequisites

- [Node.js](https://nodejs.org/) 18+
- A [Cloudflare account](https://dash.cloudflare.com/)
- Wrangler logged in: `npx wrangler login`

### 2. Install and deploy

From the repo root (PowerShell):

```powershell
cd cortex\site\workers\issues-api
npm install
npx wrangler deploy
```

First deploy creates the worker `cortex-issues-api`. Note the URL, e.g. `https://cortex-issues-api.<your-subdomain>.workers.dev`.

### 3. Set secrets (required)

Run each command and paste the value when prompted:

```powershell
npx wrangler secret put GITHUB_CLIENT_ID
npx wrangler secret put GITHUB_CLIENT_SECRET
npx wrangler secret put GITHUB_ISSUES_TOKEN
npx wrangler secret put SESSION_SECRET
npx wrangler secret put TURNSTILE_SECRET
```

Optional (open-issues sidebar on the site):

```powershell
npx wrangler secret put GITHUB_READ_TOKEN
```

| Secret | Purpose |
|--------|---------|
| `GITHUB_CLIENT_ID` / `GITHUB_CLIENT_SECRET` | OAuth app (identity only, `read:user`) |
| `GITHUB_ISSUES_TOKEN` | Fine-grained PAT with **Issues: Read and write** on `1337Xcode/cortex` only |
| `SESSION_SECRET` | Random string (32+ chars), e.g. `openssl rand -base64 32` |
| `TURNSTILE_SECRET` | Cloudflare Turnstile secret key |
| `GITHUB_READ_TOKEN` | Optional read-only PAT for `/api/community` feed |

Redeploy after setting secrets (optional but safe):

```powershell
npx wrangler deploy
```

### 4. GitHub OAuth app

[Create an OAuth App](https://github.com/settings/developers):

- **Homepage URL:** `https://1337xcode.github.io/cortex/`
- **Callback URL:** `https://cortex-issues-api.<your-subdomain>.workers.dev/auth/callback`  
  (must match your real Worker URL from step 2)

### 5. Turnstile

Create a widget at [Cloudflare Turnstile](https://dash.cloudflare.com/). Use the **site key** in the Astro site and the **secret key** as `TURNSTILE_SECRET` above.

### 6. Wire the static site

In `cortex/site/.env` for local builds, or **GitHub repository secrets** for production (required for GitHub Pages):

| Secret | Purpose |
|--------|---------|
| `PUBLIC_ISSUES_API_URL` | Worker URL from step 2 |
| `PUBLIC_TURNSTILE_SITE_KEY` | Turnstile site key (widget on issues page) |
| `TURNSTILE_SITE` | Accepted alias for `PUBLIC_TURNSTILE_SITE_KEY` |

The **Deploy Site** workflow passes these into `astro build`. Adding secrets alone is not enough until you **re-run Deploy Site** (or push to `main` under `site/**`).

```env
PUBLIC_ISSUES_API_URL=https://cortex-issues-api.<your-subdomain>.workers.dev
PUBLIC_TURNSTILE_SITE_KEY=<turnstile-site-key>
```

### 7. GitHub labels

Ensure the repo has labels: `bug`, `enhancement`, `question`, `security`.

## Local development

```powershell
cd cortex\site\workers\issues-api
copy .dev.vars.example .dev.vars
# Edit .dev.vars with your values
npm install
npm run dev
```

Worker runs at `http://localhost:8787` by default.

## Alternative: Connect repo in Cloudflare (no folder upload)

In the dashboard: **Workers & Pages → Create → Connect to Git** → select your repo → set:

- **Root directory:** `cortex/site/workers/issues-api`
- **Build command:** `npm install && npx wrangler deploy`
- **Or** use Cloudflare’s Workers build with `wrangler.toml` present (same folder)

Still configure secrets under **Settings → Variables and Secrets** in the Worker, not in git.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Dashboard “TypeScript / build process” error | Use `npx wrangler deploy`, not folder upload |
| `Not authenticated` on issues page | Set `PUBLIC_ISSUES_API_URL` and redeploy site |
| OAuth redirect mismatch | Callback URL must exactly match Worker URL + `/auth/callback` |
| `Issue submission is not configured` | Set `GITHUB_ISSUES_TOKEN` secret and redeploy |

Never commit `.dev.vars` or real tokens (see `site/.gitignore`).
