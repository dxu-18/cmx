# MAIL-DESIGN: email / DM drafter and sender (`cmx link`, `logout`, `draft`, `send`)

> **TEMPORARY FILE.** Part of the `claude/` tracking system. Remove only when the user says so.
> Status: **DESIGN PROPOSAL, nothing implemented.** Session 5, part 5, after ≥30 minutes of research (RESEARCH R12).
> Open questions for the user are in §11. Building starts after the user answers.

## Summary: the recommended design in one screen

1. **One message builder, several transports.** cmx writes a standard `.eml` (UTF-8, MIME) once. Depending on the account it goes out by:
   - SMTP via the system `curl` (Gmail/iCloud/Yahoo, app passwords);
   - Microsoft Graph (Outlook, browser sign-in);
   - Apple Mail or GNOME Online Accounts (school accounts that block everything else; Apple Mail has a quoting quirk on macOS 15+);
   - a prefilled compose window (last resort).

   No crates, no TLS code, and secrets never on a command line.
2. **Linking is guided and verified.** cmx detects the provider, opens the exact settings page, and checks the login without sending mail. The secret goes into the Keychain / Secret Service. School Google accounts are recognized up front.
3. **Always scheduled, and safe.**
   - Every send shows a preview card and goes out in 5 min unless `--override`.
   - `cmx outbox` lists, cancels or sends now. Re-sending the same file replaces the queued copy.
   - Late messages (machine was off) are held, not sent hours late.
   - Results show up in the next `cmx` command, so you never have to watch for a notification.
4. **Efficient background sending.** A launchd agent / systemd path unit is installed once and fires only while the queue is non-empty. Nothing runs while the outbox is empty.
5. **Templates.** ~30 built-in templates (academic first, then career and personal) with casual/polite/formal written per sentence, plus custom templates. `cmx draft` has a searchable picker with a live preview and field prompts.
6. **DMs and texts stay within the platforms' rules.**
   - Discord and Instagram: a hand-off (the text is on your clipboard and the right page opens; you click Message, paste and send, as you).
   - Discord: an optional own-bot mode for servers you run.
   - Texts through Messages.app if phone numbers are in scope.
   - No account-ban risks.

Open decisions for the user are in §11 (Q1–Q7).

## 0. Spec recap (prompt.txt 868–888)

Commands:
- `cmx link EMAIL`: print instructions when 2FA is on.
- `cmx link EMAIL|USERNAME/ID --discord|--instagram`.
- `cmx logout --discord|--instagram`.
- `cmx draft FILE.txt`.
- `cmx send FILE.txt [RECIPIENT] [TIME]` and `cmx send FILE.txt`.
- `cmx send "text" --discord|--instagram`: instant, if logged in.

Requirements:
1. Connect email (gmail, iCloud, yahoo, outlook) and a phone number; `cmx send` refuses until something is linked.
2. Templates for the most common occasions.
3. Casual / polite / formal variants, polite by default.
4. Custom templates, which have no variants.
5. `cmx send` **always schedules**: default now + 5 min; `--override` sends now.
6. DMs only for Instagram and Discord for now.

Constraint: zero crates (CORE). System tools are fine: `curl`, `security`, `launchctl`, `systemctl`, `osascript`, `openssl`.

## 1. What the research changed (R12)

| Finding | Consequence |
|---|---|
| Gmail, iCloud and Yahoo accept **app passwords** over SMTP; plain passwords are dead | The default `link` flow is a guided app-password setup. cmx opens the right settings page, checks the password with SMTP AUTH + NOOP (no mail sent) and stores it in the Keychain. This is exactly the spec's "if 2FA is enabled, print instructions". |
| **Outlook.com is OAuth-only** (no app passwords since 2024-09); reports since 2026-10 say **new** personal accounts reject even OAuth SMTP ("SmtpClientAuthentication is disabled for the Mailbox") | Outlook uses Microsoft sign-in: browser + PKCE by default, device code over SSH (`Mail.Send` + `offline_access`). It sends through **Microsoft Graph `POST /me/sendMail`** with the same `.eml` (base64 MIME, `Content-Type: text/plain`). That avoids SMTP AUTH entirely and files the copy in Sent Items. cmx needs a registered Microsoft app id (Q2). |
| Gmail OAuth: no device flow; SMTP needs a *restricted* scope (CASA audit); "Testing" apps get 7-day tokens | Gmail OAuth is an optional extra: the Gmail API with `gmail.send` (sensitive scope, verification) or a bring-your-own client. App password stays the default. |
| Google's help page says **work/school/organization accounts can't create app passwords at all** (support.google.com/accounts/answer/185833), and admins can block unverified OAuth apps | A school Google account (e.g. CMU's) can't use the default path. `link` says so up front, by domain/MX, and offers the escape hatches (§6: Apple Mail on macOS, GNOME Online Accounts) or Gmail OAuth with a verified cmx client (Q1). |
| curl does SMTP/IMAP with TLS, XOAUTH2, credentials over stdin (`-K -`) and login checks (`-X NOOP`) | Transport is `curl` (already on macOS and Linux). No TLS code in cmx. Secrets never touch argv. |
| iCloud doesn't file SMTP-sent mail in Sent; Gmail and Outlook do | After sending, cmx IMAP-APPENDs to "Sent Messages" (iCloud) via curl. Yahoo: check on the first real send. |
| launchd *misses* `StartInterval` firings during sleep; transient systemd timers die on reboot (default accuracy 1 min); both launchd (`QueueDirectories`) and systemd (`DirectoryNotEmpty=`) can start a job exactly while a directory is non-empty; re-registering agents risks repeated "Background Items Added" alerts | A per-user agent/unit installed once and triggered by "queue directory non-empty" (launchd `QueueDirectories`, systemd `DirectoryNotEmpty=`), plus catch-up on every `cmx` invocation. |
| Discord forbids automating user accounts; bots can DM only users who share a server (and must have connected to the gateway once) | A Discord DM defaults to a **hand-off** (sent as you, one keypress). **Your own bot** (automatic, appears as the bot, only in servers you administer) is opt-in. No user-token automation (Q3). |
| Instagram's API only lets *professional* accounts *reply* within 24 h; private APIs risk a ban | Instagram is a **hand-off** only (Q4). |
| macOS Messages is scriptable (iMessage/SMS/RCS); carrier email-to-SMS gateways are closing | "Phone number" = texting through Messages.app on macOS (KDE Connect on Linux), if that's what the spec means (Q5). |

## 2. User experience (the target transcripts)

### 2.1 Linking

```
$ cmx link you@gmail.com
Gmail. Google needs an app password for cmx (your normal password won't work):
  1. 2-Step Verification must be on → https://myaccount.google.com/signinoptions/twosv
  2. Make an app password named "cmx" → https://myaccount.google.com/apppasswords  (opened)
Paste it here (hidden): ••••••••••••••••
Checking smtp.gmail.com … ✓ signed in (no email was sent)
Your name on outgoing mail [Your Name]:
Linked you@gmail.com (default). Stored in your login Keychain.
```

- The provider is detected from the domain: a built-in table for the four named providers, then the MX record, then the Thunderbird ISPDB. MX tells custom domains apart:
  - `*.google.com` / `aspmx.l.google.com` → Google Workspace, so no app passwords: the school-account path;
  - `*.mail.protection.outlook.com` → Microsoft 365 (organization tenant);
  - `mx0[12].mail.icloud.com` → iCloud+ custom domain (the username is the Apple Account address; the From can be the custom one).
- MX and ISPDB lookups go to DNS and Mozilla; only unknown domains are looked up, and cmx says so.
- If the App passwords page doesn't exist for the account, the guide explains why (school/Workspace admin, security keys, Advanced Protection) and what still works.
- Outlook uses Microsoft sign-in instead: the browser opens, you approve, and cmx catches the redirect on 127.0.0.1. Over SSH it prints a device code: "Open microsoft.com/devicelogin and enter B7XQ-K2PM … ✓".
- `cmx link` with no argument lists linked accounts and their status: provider, auth kind, **where the secret is stored** (Keychain / Secret Service / file), default account, last successful send.
- Borrowed from `gh auth login` (cli.github.com/manual/gh_auth_login):
  - Device codes are copied to the clipboard as well as printed.
  - The credential store falls back to a 0600 file only when none exists, and says so; `--insecure-storage` forces it.
  - `--password-stdin` links without prompts (scripts, dotfiles): `pass show gmail-app | cmx link you@gmail.com --password-stdin`.
- `cmx logout EMAIL` deletes the stored secrets (and revokes OAuth tokens). `--discord` and `--instagram` work the same way.

### 2.2 Drafting

```
$ cmx draft lab4.txt
 Draft ─ pick a template                    tone: ‹ polite ›  (Tab)
 > extension        Ask for a deadline extension
   meeting          Ask for a meeting / office hours
   recommendation   Ask for a recommendation letter
   …
 ┌ preview ─────────────────────────────────────────┐
 │ Subject: Extension request: {assignment}          │
 │ Dear Professor {professor},                       │
 │ I hope you're doing well. I'm writing to ask …    │
 └───────────────────────────────────────────────────┘
professor (last name, e.g. Smith): Smith
course (e.g. 15-213): 15-213
assignment: Lab 4
…
Wrote lab4.txt. Edit it (cmx edit lab4.txt) or send it (cmx send lab4.txt).
```

- The picker uses cmx-tui: type to filter, arrows to move, Tab to cycle the tone, Enter to choose.
- Scriptable form: `cmx draft f.txt --template extension --formal --set professor=Smith …`.
- `--casual`, `--polite` and `--formal` select the variant.

### 2.3 Sending (always scheduled)

```
$ cmx send lab4.txt
  To       Prof. Smith <smith@andrew.cmu.edu>
  From     Your Name <you@gmail.com>
  Subject  Extension request: Lab 4 (15-213)
  Body     11 lines · 1 attachment (lab4-partial.pdf, 240 KB)
  Sends    today 14:05 (in 5 min)   cancel: cmx outbox cancel 3   now: cmx outbox now 3
```

- Recipient and time on the command line override the file: `cmx send lab4.txt smith@andrew.cmu.edu "tomorrow 9am"`.
- `--override` sends immediately and reports `Sent ✓`.
- **Sending the same file again while it's queued** offers to replace the queued copy: "lab4.txt is scheduled (#3, 14:05). Replace it with this version? [Y/n]". So "schedule, then keep polishing" works.
- **Results reach the user even if no notification shows:** the next `cmx` command prints one line, e.g. `✓ sent "Extension request: Lab 4" to smith@… at 14:05`, or `✗ 1 message failed — cmx outbox`. A desktop notification is best effort.

`cmx outbox` lists scheduled, held, failed and recent sent messages; `cancel N`, `now N` and `show N` act on one. (This is a CLI addition beyond the spec: Q6.)

### 2.4 DMs and texts (instant, per spec)

```
$ cmx send "running 5 min late" alice --discord
$ cmx send "on my way" +15551234567          # if phones are in scope (Q5)
```

## 3. The draft file format

```
To: Prof. Smith <smith@andrew.cmu.edu>
Cc:
Subject: Extension request: Lab 4 (15-213)
Attach: ~/hw/lab4-partial.pdf

Dear Professor Smith,
…
```

- **Header block:** an optional block before the first blank line holds `To`, `Cc`, `Bcc`, `Subject`, `From` (picks one of your linked accounts), `Attach` (repeatable) and `Reply-To`. Lines starting with `#` are comments. An unknown header is an error that suggests the closest known name.
- **Placeholders:** an unfilled `{placeholder}` blocks sending: "lab4.txt:7: {new_due} isn't filled in".
- **Recipients** can be addresses, `Name <addr>`, or contact names from `~/.cmx/contacts` (`smith = Prof. Smith <smith@andrew.cmu.edu>`). Recipients of sent mail are learned automatically.

## 4. Message construction (RFC 5322 / MIME)

- **Headers:**
  - CRLF line endings.
  - `Date` with the local offset, `Message-ID <time.random@sender-domain>`, `MIME-Version`.
  - A non-ASCII `Subject` and display names become RFC 2047 encoded-words: `=?UTF-8?B?…?=`, each ≤ 75 chars, split on whole UTF-8 characters, folded with CRLF SPACE, header lines ≤ 76 (rfc-editor.org/rfc/rfc2047; Postfix `ENC_WORD_MAX_LEN 75`).
  - CR/LF in any header value from the command line or template fields is rejected (header injection).
  - Bcc is never written as a header; it is only an extra `--mail-rcpt`.
  - No `X-Mailer` (privacy).
- **Body:** `text/plain; charset=utf-8`.
  - 7bit when the body is ASCII with lines ≤ 998; otherwise quoted-printable. Soft breaks let clients reflow long paragraphs.
  - No format=flowed: poorly supported (Fastmail's write-up).
- **Attachments:** `multipart/mixed`, base64, RFC 2231 filenames. Size checks per provider before scheduling (Gmail 25 MB, iCloud 20 MB, Yahoo 25 MB, Outlook ~20 MB).
- **Previews:** `cmx send --dry-run` prints the exact `.eml`.

## 5. Transport

- **SMTP via `curl`:**
  - URL `smtps://host:465/<sender-domain>` or `smtp://host:587/<sender-domain>` with `--ssl-reqd`. The path sets the EHLO name.
  - `-K -` on stdin carries `user`/`oauth2-bearer`.
  - `--mail-from`, then one `--mail-rcpt` per To/Cc/Bcc, `--mail-rcpt-allowfails`; per-recipient failures are parsed from `-v` (only the `RCPT`/reply pairs; the stream also holds the base64 AUTH line, so it is never logged), and `--upload-file` pointing at the 0600 `.eml` in the outbox.
- **Outlook:** Graph `POST https://graph.microsoft.com/v1.0/me/sendMail`, body = base64 of the same `.eml`, `Content-Type: text/plain`. The bearer token goes in a curl config on stdin, never argv. Returns `202 Accepted` and saves to Sent Items. Size: requests are capped at about 4 MB with base64 included, so this path takes messages up to ~2.9 MB. Larger ones try SMTP XOAUTH2 (when the mailbox allows it), else go as a draft (`POST /me/messages`) plus attachment upload sessions (3–150 MB), then `/send`.
  - SMTP XOAUTH2 (`--oauth2-bearer` + `--login-options AUTH=XOAUTH2`, scope `https://outlook.office.com/SMTP.Send`) is kept only as a fallback. It can't share one token with Graph scopes.
  - **Sign-in, default:** authorization code + PKCE (S256) with a loopback redirect. cmx listens on `127.0.0.1:<random>` with std::net, opens the browser, and reads `?code=`. The same mechanism serves Gmail OAuth later.
  - **Sign-in, fallback:** the device code flow, used over SSH or with no browser (`/{consumers|organizations}/oauth2/v2.0/devicecode`, poll `/token` every `interval` s while `authorization_pending`; the code lives 15 min). Many work/school tenants **block device code** by a Microsoft-managed Conditional Access policy (2025), so it can't be the default.
  - Refresh with `grant_type=refresh_token`.
  - A ~150-line JSON reader in cmx (no crate).
- **Sent copy:** IMAP APPEND via curl for iCloud. cmx finds the folder flagged `\Sent` with `LIST "" "*"` once at link time and caches it, then runs `imaps://imap.mail.me.com/<folder> -T msg.eml`. curl adds `(\Seen)`; success is curl's exit status, since response_code is 000 for IMAP. Gmail and Outlook file it themselves. Yahoo: decide after a real test.
- **Errors in plain words:**
  - `535` → "Gmail rejected the app password (revoked?) — run cmx link you@gmail.com".
  - `550` on a recipient → named.
  - No network → retry at 1, 5 and 15 min, then mark failed.
- **Daily limits:** sent-history counts give a warning near a provider's limit (Gmail ~500 recipients/day).

## 6. Credentials

- **macOS:** the login Keychain through `/usr/bin/security -i` (commands on stdin, so the secret is never in argv). Service `cmx`, account `<address>` / `discord-bot` / `outlook-refresh:<address>`.
- **Linux:** the Secret Service through `secret-tool store` (secret on stdin) when a provider runs. Otherwise `~/.cmx/credentials` with mode 0600 and a one-time warning, like git-credential-store. Headless VMs have no keyring (verified).
- **Hygiene:** secrets never go into logs, errors, argv or the outbox. `cmx logout` removes them.

**Escape hatches for locked-down accounts** (school Workspace accounts with app passwords and unverified apps blocked). Both reuse a sign-in the user already made in a trusted app, so cmx stores no secret:
- **macOS: `cmx link you@school.edu --via-mail-app`** sends through Apple Mail when the account is set up there. Apple Mail signs in to Google and Microsoft with its own verified OAuth client.
  - `link` sends a test message to yourself and asks you to confirm it arrived.
  - The Automation permission is granted then, while you watch.
  - Plain text only (`html content` is deprecated in Mail.sdef). Old reports of silent failures are why this is opt-in.
  - **Known defect:** since macOS 15, Mail wraps AppleScript-inserted bodies in `<blockquote type="cite">`, so the mail looks quoted or forwarded (Apple FB11734014, per github.com/sweetrb/apple-mail-mcp). The `link` self-test therefore asks "does the test message look normal (no quote bar)?". If not, cmx recommends the compose hand-off (`--handoff`) for that account instead. Inserting text into Mail's visible editor via Accessibility would avoid the bug, but it's fragile and not planned.
- **Any OS, any account: email hand-off.** `cmx link you@school.edu --handoff` records the account in hand-off mode, so the spec's rule that `send` needs a link still holds. `cmx send` (or `--handoff` on one send) then opens a prefilled compose window and you press Send. No credentials are stored. Attachments can't travel this way, so cmx says "attach lab4.pdf yourself". The window is:
  - Gmail web: `mail.google.com/mail/?view=cm&to=…&cc=…&bcc=…&su=…&body=…`;
  - Outlook web: `outlook.live.com/mail/0/deeplink/compose?to=…&subject=…&body=…` (cc/bcc unconfirmed);
  - otherwise a `mailto:` URL for the default mail app.

  This is the last resort, and the honest analogue of the DM hand-offs.
- **GNOME: GNOME Online Accounts.** If the account is in Settings → Online Accounts, `gdbus call … org.gnome.OnlineAccounts.OAuth2Based.GetAccessToken` returns a token (GOA exists to hand tokens to session apps). cmx then sends via SMTP XOAUTH2. Detected automatically during `link` on GNOME.

## 7. Outbox and scheduler

- **Outbox** (`~/.cmx/outbox/`, dir 0700, files 0600): one directory per message.
  - Contents: `msg.eml`, `meta` (account, recipients, due time, source path and hash, state, attempts) and `log`.
  - States: `scheduled → sending → sent | failed | held | cancelled`. Transitions are atomic renames.
  - Sent items are kept 30 days for `cmx outbox` and the "already sent" check.
- **At most once:**
  - The runner takes an **advisory `flock(2)`** on `outbox/lock`, which the kernel releases if the runner dies. That avoids the stuck lock files msmtp-queue's author warns about (an `O_EXCL` file outlives a killed process).
  - It marks `sending` before calling curl.
  - A crash that leaves `sending` is reported as "unknown — check your Sent folder", never silently re-sent.
- **Late messages:** if a message is more than 1 h overdue (the Mac was off or asleep), it becomes `held` and the user is told, rather than a "good morning" email going out at 7 pm. `cmx outbox now N` releases it (Q7).
- **Runner:** `cmx outbox run` is a loop: send what's due, then sleep until `min(next due, 30 s)`, re-checking the wall clock (a sleep stalls while the machine sleeps); exit when the queue is empty.
  - **The trigger is "queue directory not empty", not a timer.** The scheduled messages live in `~/.cmx/outbox/queue/`.
  - **macOS:** `~/Library/LaunchAgents/dev.cmx.outbox.plist` with `QueueDirectories = [~/.cmx/outbox/queue]` ("keeps the job alive as long as the directory is not empty") and `ThrottleInterval 15`.
    - It is installed **once**, at the first scheduled send, with `launchctl bootstrap gui/$UID`, and **never removed and re-added**. Re-adding risks repeated macOS "Background Items Added" alerts. cmx explains the one alert when it appears.
    - Not `StartInterval`: launchd *misses* those firings while the Mac sleeps (launchd.plist(5), kqueue).
  - **Linux:** `~/.config/systemd/user/cmx-outbox.path` with `DirectoryNotEmpty=%h/.cmx/outbox/queue`, which activates `cmx-outbox.service` (the same loop), enabled once. Offer `loginctl enable-linger` so mail still goes out when logged out. Fallback without systemd: a tagged crontab line `* * * * * cmx outbox run`.
  - **Runner rules both platforms need:**
    - The runner **stays alive while anything is queued**: launchd would respawn it, and systemd path triggers are rate-limited (`TriggerLimitIntervalSec` defaults to 2 s; hitting it fails the unit).
    - New items are written in `outbox/tmp/` and **renamed into `queue/`** only when complete, so a half-written message never triggers a run. (systemd also ignores dotfiles in watched dirs.)
    - At boot or login, both triggers fire at once if the queue is already non-empty.
  - **Catch-up:** every `cmx` invocation also flushes due items (cheap), so nothing depends on the trigger alone.
- **Configurable delay:** the default send delay is 5 min per spec; `~/.cmx/mail.conf` may set `delay = 2m` etc. `--override` still means now.
- **The editor stays fast:** the catch-up flush runs on CLI commands; `cmx edit`/`workspace` do it on a background thread after the first frame (a status-bar result only).
- **Efficiency:** nothing runs while the queue is empty: no polling, no timer. One curl process per send.
- **Robustness:**
  - The agent/unit files record `std::env::current_exe()`. Each `send` compares it with the running binary and rewrites the file **only if the path changed** (a reinstall or move), so a stale path never strands the queue without churning the registration.
  - The macOS agent lives in the `gui/$UID` domain, so it runs while you're logged in, and the login Keychain is unlocked then. Items created by `/usr/bin/security` are read by the same tool, so there's no ACL prompt.
  - With no scheduler at all (containers, WSL without systemd, no cron), `send` says so honestly: messages go out the next time any `cmx` command runs.

## 8. Templates

**Built-in library** (≈30; academic first, since the user is a student):

| Group | Templates |
|---|---|
| Academic | extension, meeting/office-hours, course question, absence notice, recommendation request, recommendation thank-you, regrade request, research inquiry (cold email), TA application, waitlist/enrollment, follow-up (no reply) |
| Career / work | application, networking/coffee chat, informational interview, interview thank-you, post-interview follow-up, accept offer, decline offer, referral request, reschedule, resignation, introduction (connect two people), reminder, confirmation, meeting recap |
| Personal | thank-you, apology, congratulations, invitation, accept/decline invitation, check-in, condolences |

**Format:** one source per template. Variants are chosen per line; a full example is in §10a. The header comment names the template and its fields (`# field professor: Professor's last name (e.g. Smith)`), and these drive the draft prompts.

- **Variant lines:** a line tagged `@casual` / `@polite` / `@formal` (or `@polite,formal`) appears only in that variant. Untagged lines are shared. A small difference can be inline: `{casual:Thanks!|polite:Thank you.|formal:Thank you for your consideration.}`.
- **Shared fields:**
  - `{me}`: your name from link.
  - `{close}`: Best, / Best regards, / Sincerely, by tone.
  - `{today}` and `{signature}` (`~/.cmx/signature.txt`).
- **Custom templates** live in `~/.cmx/templates/NAME.txt`: the same format without `@` variants (per spec). They appear in the picker under "Yours".

## 9. DMs and texts

| Channel | Mode | How | Limits |
|---|---|---|---|
| Discord | **hand-off** (default) | Copies the text (pbcopy / wl-copy / xclip) and opens `https://discord.com/users/<id>` (app if installed, else browser), then says "click Message, ⌘V, ↵". Without a bot cmx can't look up a username, so the numeric id comes from `~/.cmx/contacts` (`alice = discord:123456789012345678`, from Developer Mode → Copy User ID; cmx asks once and remembers). With no id, it opens `https://discord.com/channels/@me` and you pick the chat. | Sent as you, but needs a click and a paste. Works for anyone. |
| Discord | **bot** (opt-in, automatic) | `link --discord` walks through making your own bot (Developer Portal: New Application → Bot → token), pastes the token (Keychain), opens the invite link for a server you share with your friends, and does the one-time gateway IDENTIFY. That uses `curl wss://` where curl has ws (Linux), else `openssl s_client` plus a WebSocket upgrade written in cmx. LibreSSL checks the chain but not the hostname, so cmx checks the leaf certificate's names itself before sending the token (R12). `send` resolves `alice` with Search Guild Members (no privileged intent), then Create DM + Create Message (REST via curl). | The message comes **from the bot** ("<your name>'s cmx"). The recipient must share a server with it and allow DMs from server members. **Only a server's owner or a member with Manage Server can add a bot**, so this mode suits people who run their own server. |
| Instagram | **hand-off** only | Copies the text and opens the profile `https://www.instagram.com/<username>/` (click **Message**, paste, send). Meta says `ig.me/m/…` links aren't supported on Instagram Web; on a phone they would open the thread directly. | The official API can't start personal DMs, and private APIs risk a ban. |
| Phone (if in scope) | Messages.app (macOS) | `osascript`: `send "…" to participant "+1…" of (1st account whose service type = iMessage)` (compiles against Messages.sdef on macOS 27.2), with SMS through iPhone forwarding. Permission is granted once during `link` while you watch. The text reaches the script through a 0600 file the script reads, never argv (visible in `ps`). | No delivery receipt. Linux: `kdeconnect-cli --send-sms` when an Android phone is paired. |

Rejected:
- Discord user-token "self-bot": against the ToS, account termination.
- Discord Social SDK DMs: approval-gated, aimed at games.
- Instagram private APIs: ToS, ban risk.
- Carrier email-to-SMS: shutting down.
- Apple Mail scripting as the *main* email path: flaky reports. It stays the opt-in escape hatch `--via-mail-app` (§6).

## 10. Code layout and phases

- New crate `cmx-mail` (zero deps):
  - `account` (detect / link / logout / store), `message` (draft parse, MIME), `smtp` (curl), `oauth` (PKCE loopback, device code, refresh, JSON), `outbox`, `schedule` (launchd / systemd / cron), `when` (time parser), `templates` (library + variant engine), `dm` (discord, instagram, messages).
- The CLI in `crates/cmx/src/main.rs`: new arms `link`, `logout`, `draft`, `send`, `outbox` in the top-level `match`. The picker uses `cmx-tui`.
- `cmx-sys` additions:
  - `time`, `localtime_r`, `mktime` (`struct tm`'s 9 ints + `tm_gmtoff` + `tm_zone` share a layout on macOS and 64-bit glibc).
  - Hidden input uses the existing termios raw mode (read bytes until Enter, handling ⌫ and bracketed paste, no echo).
- **Time:** the grammar is in §10a. Local time zone via libc `localtime_r` / `mktime` (cmx-sys FFI, `tm_isdst = -1` for DST).
- **Tests:**
  - A Rust fake SMTP server in tests (transcript and `.eml` checks).
  - MIME round-trips.
  - A fake HTTP server for OAuth / Discord.
  - The time parser with a fixed clock.
  - The scheduler against temporary directories (env override), plus the Linux VM for systemd.
- **Phases:**
  1. Message, draft format, MIME, time parser, outbox, `--override` SMTP with app passwords (Gmail/iCloud/Yahoo), credential stores, `link` with verification.
  2. Scheduler, runner, held/late policy, in-band results, notifications.
  3. Templates and the `draft` picker.
  4. Outlook: Microsoft sign-in (PKCE loopback, device-code fallback) plus Graph sendMail (needs the app id, Q2).
  5. DMs: Discord bot and hand-off, Instagram hand-off, Messages.
  6. Gmail OAuth (optional): PKCE loopback, `gmail.send`, and the raw `.eml` media upload to `/upload/gmail/v1/users/me/messages/send` (≤ 35 MB).

## 10a. Details worked out during the brainstorm

**Time grammar** (`when`). The resolved time is always echoed as weekday, date and time, plus "in 3h 10m".

| Input | Meaning |
|---|---|
| (nothing) | now + 5 min (spec) |
| `in 10m`, `in 2h`, `in 1h30m`, `+45m` | relative |
| `5pm`, `5:30pm`, `17:30` | the next time it is that time today, otherwise tomorrow |
| `9` (a bare hour) | **ambiguous**: asks "9 AM or 9 PM?" (non-interactive: error). Never guessed. |
| `noon`, `midnight` | 12:00, 00:00 (next) |
| `tomorrow` | tomorrow 9:00 AM (the common app convention) |
| `tomorrow 8am`, `today 6pm` | as said |
| `morning` / `afternoon` / `evening` / `tonight` | 9:00 / 13:00 / 18:00 / 20:00, today if still ahead, else tomorrow |
| `fri`, `friday 8am` | the soonest Friday ahead (today if the time is still ahead) |
| `next fri` | Friday of **next** week (the OmniFocus convention, "this" = this week); echoed as a date so a misreading is caught |
| `2026-11-20 08:00`, `11/20 8am`, `nov 20 8am` | absolute (US month/day order; ISO always works) |
| a past time | an error, unless `--override` |

`[RECIPIENT]` and `[TIME]` are told apart by shape: an `@`, a phone number, or a known contact is a recipient; anything that parses as a time is a time. So `cmx send lab4.txt tomorrow 9am` works without a recipient.

**Outbox record** (`~/.cmx/outbox/0003/meta`, `key: value` lines like mail headers, read and written by cmx):

```
id: 3
state: scheduled
account: you@gmail.com
to: smith@andrew.cmu.edu
subject: Extension request: Lab 4 (15-213)
due: 2026-11-14T14:05:00-05:00
created: 2026-11-14T14:00:12-05:00
source: /Users/…/lab4.txt
source-hash: fnv1a:9c2e…
attempts: 0
```

The `.eml` sits next to it. A send moves the directory to `sent/` (kept 30 days) or `failed/`.

**One template written out in full** (checks that the per-line variant format reads well):

```
# extension — Ask for a deadline extension
# field professor: Professor's last name (e.g. Smith)
# field course: Course (e.g. 15-213)
# field assignment: Assignment (e.g. Lab 4)
# field due: Current deadline (e.g. Friday at 11:59pm)
# field new_due: The deadline you're asking for (e.g. Monday at noon)
# field reason: One or two sentences on why
Subject: Extension request: {assignment} ({course})

@casual Hi Professor {professor},
@polite Dear Professor {professor},
@formal Dear Professor {professor},

@casual Hope your week is going well! I'm in {course}, and I wanted to ask about {assignment}, which is due {due}.
@polite I hope you're doing well. I'm a student in {course}, and I'm writing to ask whether I could have an extension on {assignment}, currently due {due}.
@formal I hope this message finds you well. I am a student in {course}, and I am writing to respectfully request an extension on {assignment}, which is currently due {due}.

{reason}

@casual Would it be okay if I turned it in by {new_due}? I can send what I have so far if that helps.
@polite Would it be possible to submit it by {new_due}? I'm happy to share my progress so far.
@formal Would it be possible to submit the assignment by {new_due}? I would be glad to share my current progress.

@casual Thanks so much!
@polite Thank you for your time and understanding.
@formal Thank you very much for your time and consideration.

{close}
{me}
```

Here `{close}` is "Best," / "Best regards," / "Sincerely,". Each sentence is written per tone, not just the greeting: a formal email changes contractions and verbs too ("I'm" → "I am", "turn in" → "submit"). Sources:
- Duke ARC's email guide gives the meeting and recommendation structure.
- Surveys show "Hi" and "Kind regards" are the most liked; "Cheers" is avoided.

**Tone rules for writing the built-in templates** (so all ~30 are consistent):

| | casual | polite (default) | formal |
|---|---|---|---|
| Greeting | `Hi {first},` | `Hi {title} {last},` / `Dear …` for professors | `Dear {title} {last},` |
| Opener | "Hope you're doing well!" | "I hope you're doing well." | "I hope this message finds you well." |
| Contractions | yes | yes | no |
| Request | direct: "Could I …?", "Would it be okay if …?" | modal: "Would it be possible to …?" | modal + if-clause + hedge: "I would be grateful if …", "Would it be possible to …" |
| Hedges (possibly, perhaps) | none | at most one | one or two |
| Exclamation marks | at most one | none | none |
| Thanks | "Thanks so much!" | "Thank you for your time." | "Thank you very much for your time and consideration." |
| Closing (`{close}`) | `Best,` / `Thanks,` | `Best regards,` | `Sincerely,` / `Kind regards,` |

Sources (R12): university writing guides on modals, if-clauses and hedging (edtechbooks "Formal Emails"); the Ibérica 2008 email corpus; the sign-off surveys. "Cheers" and "Love" are never used.

**Phrasing variety (later):** the stex phrase-bank machinery (weights, `--natural/--varied`) could vary sentences inside a tone, so the fifth extension request doesn't read identical to the first. Off by default; templates stay deterministic.

**cmx editor integration (later):** the status bar shows `✉ 1 · 14:05` while something is scheduled, and `.txt` drafts get header/placeholder highlighting.

## 10b. Risks and mitigations

| Risk | Mitigation |
|---|---|
| A provider changes auth again (Yahoo app passwords "temporarily unavailable"; Microsoft blocking SMTP AUTH for new accounts) | One message builder, several transports (SMTP, Graph, Gmail API, Mail.app). `link` verifies at once, and errors name the fix. |
| Double send after a crash | At-most-once state machine; an unclear outcome is reported, never retried blindly. |
| Wrong recipient or wrong time | The preview card on every `send`, the 5-minute default window, `cmx outbox cancel`, time echoed as a date, bare hours never guessed. |
| Late send after the laptop slept or was off | `held` state beyond 1 h (Q7). |
| Secret leakage | Keychain/Secret Service; stdin-only to `curl`/`security`; `-v` output parsed for RCPT lines only; files 0600, dirs 0700. |
| TLS without crates | curl's own verification for every send. The single `openssl s_client` use (Discord IDENTIFY) checks the chain with `-verify_return_error` and the hostname in cmx. |
| macOS permission prompts at odd times | Every Apple-event path is exercised once during `link` while you watch. Background sends use only curl and security, which need no TCC. |
| curl missing (minimal Linux) | A clear error naming the package. curl is preinstalled on macOS and on common desktop distros. |

## 11. Open questions for the user

1. **Q1 Accounts.** Which accounts do you want first: personal Gmail, your school account, iCloud, Outlook? School accounts are the hard case:
   - A school *Google* account can't use app passwords at all (Google's rule for organization accounts).
   - A school *Microsoft 365* account often needs IT approval for mail apps.
   - Their routes are Apple Mail (if the account is set up there; but macOS 15+ makes scripted mail look quoted, see §6), a verified Gmail OAuth app for cmx (Google's sensitive-scope verification: officially 3–5 business days, though 2026 forum reports show privacy-policy reviews stalling for weeks; needs a privacy policy, a verified domain and a demo video; no paid audit for `gmail.send`), or the prefilled-compose hand-off.
   - Personal Gmail, iCloud and Yahoo work directly with app passwords.
2. **Q2 Outlook.** Outlook.com needs a Microsoft app registration (free, ~5 min in the Entra portal; needs an Azure account, which may ask for card/phone verification; public client with no secret, personal accounts enabled, delegated `Mail.Send`; steps in Appendix A). Will you register one for cmx (I'll write the exact steps), or should cmx only accept a user-provided app id?
3. **Q3 Discord.** Options are hand-off (sent as you, one keypress; my recommended default) or your own bot (automatic, but it appears as the bot, and recipients must be in a server where you can add it, which needs owner/Manage Server rights). Or both, falling back to hand-off when the bot can't reach someone. The user-token approach that "sends as you" automatically is against Discord's terms. I recommend not building it.
4. **Q4 Instagram.** No compliant automatic DM exists for personal accounts. Is hand-off acceptable?
5. **Q5 Phone number.** Does "connect email and phone number" mean texting (iMessage/SMS through Messages.app) as a send channel, or something else, such as a number needed for 2FA?
6. **Q6 CLI.** Is `cmx outbox [cancel|now|show N]` acceptable as an addition to the spec's commands?
7. **Q7 Late sends.** Hold messages that are more than 1 h overdue (machine was off) instead of sending them late? Is 1 h the right threshold?

## Appendix A. Registering the Microsoft app (for Q2)

Steps are from learn.microsoft.com/entra/identity-platform/quickstart-register-app, updated 2026-06.

1. Sign in at https://entra.microsoft.com. **Prerequisite:** an Azure account. The free one usually wants card/phone verification, and this is the main friction.
2. Go to **Entra ID → App registrations → New registration**. Name it `cmx`.
3. **Supported account types:** choose **Any Entra ID Tenant + Personal Microsoft accounts**. That covers Outlook.com/Hotmail and school/work Microsoft 365.
4. Add a redirect URI: **Authentication → Add a platform → Mobile and desktop applications → `http://127.0.0.1`**. Microsoft ignores the port for loopback, so cmx can use any free port, but the path must match: cmx redirects to the root `/`. `[::1]` isn't supported.
5. Turn on **Allow public client flows** (needed for the device code fallback). There is no client secret.
6. **API permissions → Microsoft Graph → Delegated:** `Mail.Send`, `offline_access`, plus `User.Read` (the default).
7. Copy the **Application (client) ID** from Overview. It is public and gets compiled into cmx.

Unverified publishers: personal accounts can consent. School and work tenants often show **"Need admin approval"** for mail scopes like `Mail.Send` (user consent off, or Microsoft's default "recommended" consent policy since Aug 2025). There the escape hatches apply: Apple Mail (Outlook accounts work there too), or ask IT to approve cmx. Publisher verification is free but needs a Microsoft Partner Network id.

## Appendix B. The words `cmx link` prints (drafts)

- **Gmail (personal):**
  ```
  Gmail needs an app password for cmx; your Google password won't work here.
    1. 2-Step Verification must be on:   https://myaccount.google.com/signinoptions/twosv
    2. Create an app password ("cmx"):   https://myaccount.google.com/apppasswords   ← opened in your browser
  Paste the 16-letter password (hidden; spaces are fine):
  ```
  If the page says app passwords aren't available, the cause is one of: a work/school account, security-key-only 2-Step Verification, or Advanced Protection. cmx names the escape hatches (§6).
- **School Google account** (detected by MX or domain): "School and work Google accounts can't use app passwords. Options: send through Apple Mail (`--via-mail-app`, macOS), GNOME Online Accounts (Linux), or `--handoff` to open a prefilled Gmail compose window."
- **iCloud:** "iCloud needs an app-specific password: https://account.apple.com → Sign-In and Security → App-Specific Passwords → Generate → name it cmx. Paste it (xxxx-xxxx-xxxx-xxxx):". Path confirmed in support.apple.com/102654. That page also notes that changing your Apple password revokes app-specific passwords; cmx explains a sudden 535 that way.
- **Yahoo:** "Yahoo Account Security → Generate app password (https://login.yahoo.com/account/security) → name it cmx. Paste it:"
- **Outlook:** "Outlook signs in with Microsoft: your browser will open. Approve 'cmx' and come back here." Over SSH: "Open https://microsoft.com/devicelogin and enter B7XQ-K2PM (copied)."
- **Discord (bot mode):**
  1. https://discord.com/developers/applications → New Application → name it ("<you>'s cmx").
  2. Bot → Reset Token → copy it (shown once). Paste here (hidden).
  3. Installation → Guild Install → scopes `bot` → permission Send Messages → open the install link → add it to a server you run that your friends are in.

  cmx then connects once to Discord's gateway (required before a new bot can send) and reports "ready".

