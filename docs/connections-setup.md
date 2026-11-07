# Setting up Google and Microsoft sign-in (Settings › Connections)

openbot connects Gmail / Google Calendar and Outlook mail / calendar through its own sign-in apps:
one Google Cloud project and one Microsoft app registration. These are made once by whoever builds
openbot; everyone who uses that build then just clicks **Connect Google** or **Connect Microsoft**.

The IDs go into the build as GitHub repository secrets (never into the code):

| Secret | Where it comes from |
|---|---|
| `OPENBOT_GOOGLE_CLIENT_ID` | Google Cloud › Google Auth Platform › Clients › your Desktop client |
| `OPENBOT_GOOGLE_CLIENT_SECRET` | same client (Google requires it even for desktop apps; it can't be kept secret inside an app and Google knows that) |
| `OPENBOT_MICROSOFT_CLIENT_ID` | Microsoft Entra › App registrations › your app › Application (client) ID |

For development you can instead put them in `config.json` on the computer your Bots run on:
`"oauth": {"google_client_id": "…", "google_client_secret": "…", "microsoft_client_id": "…"}`
(the secret is encrypted when openbot saves the file).

## Why people don't have to sign in again

- **Google** gives openbot a refresh token. Google Cloud projects whose app is left in **Testing**
  get tokens that expire after 7 days, so the app must be set to **In production**
  ([Google: refresh token expiration](https://developers.google.com/identity/protocols/oauth2#expiration)).
  Unused tokens also expire after 6 months; openbot renews every account's sign-in when it hasn't
  been used for a week.
- **Microsoft** refresh tokens last 90 days and are replaced by a new one every time they're used
  ([Microsoft: refresh tokens](https://learn.microsoft.com/en-us/entra/identity-platform/refresh-tokens)).
  openbot saves the newest one and renews weekly, so it doesn't run out.
- Either one stops working if the person changes their password (some cases), removes openbot's
  access from their account, or an admin revokes it. The account then shows **Signed out** in
  Settings › Connections with a **Reconnect** button.
- Google accounts connected before a feature was added (Google Drive, other calendars and free time),
  or where a box was unticked on the permission screen, show **Update permissions**. It runs the same
  sign-in again; openbot keeps what Google reports the account allowed (`scopes` in config.json) and
  updates it on every token refresh.
- If a Google API isn't enabled in the project, Bots get an error with the link that turns it on.

## Google

1. Open [console.cloud.google.com](https://console.cloud.google.com), create a project named `openbot`.
2. **APIs & Services › Library**: enable **Gmail API**, **Google Calendar API** and **Google Drive API**.
3. **Google Auth Platform › Branding**: app name `openbot`, your support email, developer contact email.
4. **Audience**: user type **External**.
5. **Data access › Add or remove scopes**: `openid`, `.../auth/userinfo.email`, `.../auth/userinfo.profile`,
   `https://www.googleapis.com/auth/gmail.modify`, `https://www.googleapis.com/auth/calendar.events`,
   `https://www.googleapis.com/auth/calendar.calendarlist.readonly`,
   `https://www.googleapis.com/auth/calendar.events.freebusy`, `https://www.googleapis.com/auth/drive`.
6. **Clients › Create client**: application type **Desktop app**, name `openbot`. Copy the client ID
   and client secret. (Desktop clients accept any `http://127.0.0.1:<port>` return address, which
   openbot uses.)
7. **Audience › Publish app** → **In production**. Don't skip this (see above).

Until Google verifies the app, people see a "Google hasn't verified this app" screen once
(Advanced › Go to openbot), and at most 100 people can connect. Gmail and Drive access are
"restricted" scopes, so going past 100 people needs Google's verification, including a yearly security
assessment by a Google-approved company
([Google: when verification is needed](https://support.google.com/cloud/answer/13464323)).

## Microsoft

1. Open [entra.microsoft.com](https://entra.microsoft.com) › **App registrations › New registration**.
   (With a personal Microsoft account you may first be asked to create a free Azure account.)
2. Name `openbot`. Supported account types: **Accounts in any organizational directory and personal
   Microsoft accounts**.
3. Redirect URI: platform **Public client/native (mobile & desktop)**, URI `http://localhost/oauth/callback`.
   (Microsoft lets desktop apps use any port on `localhost`.)
4. **Authentication**: **Allow public client flows** = Yes.
5. **API permissions › Add › Microsoft Graph › Delegated**: `Mail.ReadWrite`, `Mail.Send`,
   `Calendars.ReadWrite`, `User.Read`, `offline_access`, `openid`, `email`, `profile`.
6. Copy the **Application (client) ID** from **Overview**.

Work or school accounts may need their IT admin to approve openbot the first time.

## How signing in works

1. In Settings › Connections the app asks the computer your Bots run on for a sign-in link
   (`POST /api/accounts/connect`). That computer makes a one-time code check (PKCE) and remembers it.
2. The link opens in your browser. After you allow access, the browser returns to
   `http://127.0.0.1:<port>/oauth/callback` — the openbot app on the device you're using.
3. That page passes the code to the computer your Bots run on (`POST /api/accounts/finish`), which
   trades it for the account's tokens and saves the refresh token encrypted in its `config.json`.
4. If you use openbot in a plain web browser (no desktop app), step 2 ends on an error page;
   copy that page's address into the box in Settings › Connections instead.
