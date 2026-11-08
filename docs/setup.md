# kwek: one-time Google Cloud setup

kwek talks to Google with your own OAuth client in your own Google Cloud project, inside your Google Workspace. Setting that up takes about 15 minutes, once. You need a Workspace account (the Chat API does not serve personal Gmail accounts) and rights to create a project, or an existing project to use.

The commands below use `P` for your project ID.

## 1. Project, APIs, Pub/Sub

Create a project (or pick an existing one) and link a billing account. kwek's usage stays inside the free tiers; a budget alert at a small amount under Billing, Budgets & alerts is a cheap safety net.

```sh
P=your-project-id
gcloud services enable chat.googleapis.com workspaceevents.googleapis.com pubsub.googleapis.com people.googleapis.com --project=$P
gcloud pubsub topics create chat-events --project=$P
gcloud pubsub topics add-iam-policy-binding chat-events --project=$P \
  --member=serviceAccount:chat-api-push@system.gserviceaccount.com --role=roles/pubsub.publisher
```

The topic carries live events (new messages, edits, reactions, read state) to the app. Keep the name `chat-events`: the downloaded app looks for it. (A build of your own can use another name, with `KWEK_PUBSUB_TOPIC=<name>`.)

### If the publisher grant is refused

Organisations with domain-restricted sharing (`iam.allowedPolicyMemberDomains`) refuse Google's service agent. An org policy admin can allow it for this project, add the binding, and remove the override again; the effective policy then returns to the org's:

```sh
printf 'constraint: constraints/iam.allowedPolicyMemberDomains\nlistPolicy:\n  allValues: ALLOW\n' > /tmp/allow.yaml
gcloud resource-manager org-policies set-policy /tmp/allow.yaml --project=$P
# retry for a minute or two until it stops failing with FAILED_PRECONDITION
gcloud pubsub topics add-iam-policy-binding chat-events --project=$P \
  --member=serviceAccount:chat-api-push@system.gserviceaccount.com --role=roles/pubsub.publisher
gcloud resource-manager org-policies delete iam.allowedPolicyMemberDomains --project=$P
```

## 2. OAuth (console, Google Auth Platform)

In https://console.cloud.google.com with your project selected, open **Google Auth Platform**.

1. **Branding**: app name `kwek`, your email as support and developer contact.
2. **Audience**: user type **Internal** (only people in your Workspace can sign in; no Google verification needed).
3. **Data access**, add these scopes:
   - `https://www.googleapis.com/auth/chat.spaces.readonly`
   - `https://www.googleapis.com/auth/chat.messages`
   - `https://www.googleapis.com/auth/chat.messages.reactions`
   - `https://www.googleapis.com/auth/chat.memberships.readonly`
   - `https://www.googleapis.com/auth/chat.users.readstate`
   - `https://www.googleapis.com/auth/pubsub`
   - `https://www.googleapis.com/auth/directory.readonly`
   - `https://www.googleapis.com/auth/userinfo.profile` and `userinfo.email` (who is signed in; Internal apps may request them without listing them)
4. **Clients**: create client, type **Desktop app**, name `kwek desktop`. Download the JSON and keep it: kwek asks for this client file on first launch.

## 3. Chat app configuration (console)

APIs & Services, Google Chat API, **Configuration** tab. Live events are refused until this exists; without it kwek falls back to polling once a minute.

- App name `kwek`, avatar URL any public https PNG (e.g. `https://developers.google.com/chat/images/quickstart-app-avatar.png`), description `Desktop client`.
- Interactive features: **off**.
- Visibility: specific people, the accounts that will use kwek.
- Save.

## 4. Start kwek

Open kwek, choose the client file from step 2 (or paste its contents), and press **Connect to Google**; that opens a browser tab once to sign in. kwek keeps the client file's details in its data folder, so this happens once.

Building kwek yourself with the file saved as `secrets/oauth-client.json` in your checkout bakes it into the app instead, and the first step is skipped (see `development.md`).

Everyone who uses kwek from the same project shares the one topic. kwek drops events that are not for the signed-in user, but they still reach each device's subscription, so for more than one user, give each their own project or topic (see `api-findings.md`).
