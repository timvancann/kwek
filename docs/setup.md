# kwek: one-time Google Cloud setup

Done once by Tim, in the `example.com` Workspace. Takes about 15 minutes. The app does everything after this.

## 1. Project, APIs, Pub/Sub (done 2026-11-07)

kwek lives in the existing **`your-project`** project (org `<org-id>`, billing `<billing-account>`). A dedicated project was not possible: the billing account is at its quota of five billed projects.

Done with gcloud, outside the infra repo's OpenTofu code:

- Enabled `chat`, `workspaceevents`, `pubsub` and `people` APIs.
- Created topic `projects/your-project/topics/chat-events`.
- Granted `chat-api-push@system.gserviceaccount.com` the Publisher role on that topic.

The org enforces domain-restricted sharing (`iam.allowedPolicyMemberDomains`, customer `<customer-id>`), which refuses Google's service agent. The grant was made by setting a project-level `allValues: ALLOW` override, adding the binding, and deleting the override again; the effective policy is back to the org's. Tim has `roles/orgpolicy.policyAdmin` on the org since then. To redo it:

```sh
P=your-project
printf 'constraint: constraints/iam.allowedPolicyMemberDomains\nlistPolicy:\n  allValues: ALLOW\n' > /tmp/allow.yaml
gcloud resource-manager org-policies set-policy /tmp/allow.yaml --project=$P
# retry for ~1-2 min until it stops failing with FAILED_PRECONDITION
gcloud pubsub topics add-iam-policy-binding chat-events --project=$P \
  --member=serviceAccount:chat-api-push@system.gserviceaccount.com --role=roles/pubsub.publisher
gcloud resource-manager org-policies delete iam.allowedPolicyMemberDomains --project=$P
```

Optional: a budget alert at EUR 1 under Billing, Budgets & alerts.

## 2. OAuth (console, Google Auth Platform)

In https://console.cloud.google.com with **your-project** selected, open **Google Auth Platform**.

1. **Branding**: app name `kwek`, your email as support and developer contact.
2. **Audience**: user type **Internal**.
3. **Data access**, add these scopes:
   - `https://www.googleapis.com/auth/chat.spaces.readonly`
   - `https://www.googleapis.com/auth/chat.messages`
   - `https://www.googleapis.com/auth/chat.messages.reactions`
   - `https://www.googleapis.com/auth/chat.memberships.readonly`
   - `https://www.googleapis.com/auth/chat.users.readstate`
   - `https://www.googleapis.com/auth/pubsub`
   - `https://www.googleapis.com/auth/directory.readonly`
   - `https://www.googleapis.com/auth/userinfo.profile` and `userinfo.email` (who is signed in; Internal apps may request them without listing them)
4. **Clients**: create client, type **Desktop app**, name `kwek desktop`. Download the JSON and save it as `~/repos/private/kwek/secrets/oauth-client.json`. That folder is gitignored.

## 3. Chat app configuration (console)

APIs & Services, Google Chat API, **Configuration** tab. User-auth calls are refused until this exists.

- App name `kwek`, avatar URL any public https PNG (e.g. `https://developers.google.com/chat/images/quickstart-app-avatar.png`), description `Desktop client`.
- Interactive features: **off**.
- Visibility: specific people, `you@example.com`.
- Save.

## 4. A test space (Google Chat web)

Create a space called `kwek-test` with only yourself in it. During development the agent sends messages, edits and reactions only there.

## 5. Tell the agent

That `secrets/oauth-client.json` is in place and the Chat app is configured.

Later, when the spike first runs, it opens a browser tab once to sign in. That one click is the only other thing needed from you.
