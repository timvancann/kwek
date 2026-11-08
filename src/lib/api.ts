// Typed wrappers over the Tauri commands and events. The webview holds no tokens and makes
// no network requests: everything goes through these calls into Rust.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AuthState } from "./bindings/AuthState";
import type { GoogleSetupView } from "./bindings/GoogleSetupView";
import type { MessageView } from "./bindings/MessageView";
import type { SearchResult } from "./bindings/SearchResult";
import type { SpaceSummary } from "./bindings/SpaceSummary";
import type { SyncStatus } from "./bindings/SyncStatus";
import type { UserRef } from "./bindings/UserRef";

export type { AuthState, GoogleSetupView, MessageView, SearchResult, SpaceSummary, SyncStatus, UserRef };

export const api = {
  googleSetup: () => invoke<GoogleSetupView | null>("google_setup"),
  setGoogleSetup: (json: string) => invoke<GoogleSetupView>("set_google_setup", { json }),
  authStatus: () => invoke<AuthState>("auth_status"),
  login: () => invoke<void>("login"),
  logout: () => invoke<void>("logout"),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  refresh: () => invoke<void>("refresh"),
  listSpaces: () => invoke<SpaceSummary[]>("list_spaces"),
  getMessages: (space: string, before: number | null, limit: number) =>
    invoke<MessageView[]>("get_messages", { space, before, limit }),
  loadOlder: (space: string) => invoke<number>("load_older", { space }),
  getThread: (thread: string) => invoke<MessageView[]>("get_thread", { thread }),
  /** `attachments` are ids from `stageAttachment`. */
  sendMessage: (space: string, thread: string | null, text: string, attachments: string[] = []) =>
    invoke<MessageView>("send_message", { space, thread, text, attachments }),
  /** Keeps an image for sending; the raw bytes cross as the request body. Returns its id. */
  stageAttachment: async (file: File) =>
    invoke<string>("stage_attachment", new Uint8Array(await file.arrayBuffer()), {
      headers: { "x-name": encodeURIComponent(file.name), "x-type": file.type },
    }),
  unstageAttachment: (id: string) => invoke<void>("unstage_attachment", { id }),
  retrySend: (name: string) => invoke<void>("retry_send", { name }),
  discardFailed: (name: string) => invoke<void>("discard_failed", { name }),
  editMessage: (name: string, text: string) => invoke<void>("edit_message", { name, text }),
  deleteMessage: (name: string) => invoke<void>("delete_message", { name }),
  toggleReaction: (message: string, emoji: string) => invoke<void>("toggle_reaction", { message, emoji }),
  markRead: (space: string, upTo: number) => invoke<void>("mark_read", { space, upTo }),
  markThreadRead: (thread: string, upTo: number) => invoke<void>("mark_thread_read", { thread, upTo }),
  search: (query: string, limit = 50, offset = 0) => invoke<SearchResult>("search", { query, limit, offset }),
  listMembers: (space: string) => invoke<UserRef[]>("list_members", { space }),
  listSenders: () => invoke<UserRef[]>("list_senders"),
  setSpacePrefs: (space: string, prefs: { muted?: boolean; starred?: boolean }) =>
    invoke<void>("set_space_prefs", { space, muted: prefs.muted ?? null, starred: prefs.starred ?? null }),
  setOpen: (space: string | null, thread: string | null) => invoke<void>("set_open", { space, thread }),
  lastOwnMessage: (space: string, thread: string | null) =>
    invoke<MessageView | null>("last_own_message", { space, thread }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  downloadAttachment: (resource: string, name: string) =>
    invoke<string>("download_attachment", { resource, name }),
};

export interface UpsertPayload {
  space: string;
  messages: MessageView[];
}
export interface DeletePayload {
  space: string;
  names: string[];
}
export interface NavPayload {
  space: string;
  message?: string | null;
  thread?: string | null;
}
/** A DM or mention that arrived while kwek is in front. */
export interface InAppNotice extends NavPayload {
  title: string;
  body: string;
}

export function on<T>(event: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => cb(e.payload));
}

/** Command errors arrive as plain sentences from Rust. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "Something went wrong.";
}
