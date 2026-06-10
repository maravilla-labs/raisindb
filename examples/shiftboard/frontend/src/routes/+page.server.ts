/**
 * SSR load + auth form actions for the shift board page.
 *
 * load: when authed, fetch the full board (shifts + staff) and the latest
 * agent conversation's message history via SQL over HTTP — the page renders
 * complete HTML before any JavaScript runs (view-source shows shift titles).
 *
 * actions:
 *   ?/login  - POST /auth/{repo}/login, store tokens in httpOnly cookies
 *   ?/logout - clear the cookies
 *
 * Both work as plain HTML form posts (no client JS required); SvelteKit
 * re-runs `load` after an action, so a successful login renders the board.
 */
import { fail, redirect } from '@sveltejs/kit';
import type { ChatMessage, ConversationListItem } from '@raisindb/client';
import type { Actions, PageServerLoad } from './$types';
import {
  REPOSITORY,
  clearAuthCookies,
  createConversationManager,
  createHttpClient,
  listPendingTasks,
  loginWithEmail,
  setAuthCookies,
} from '$lib/server/raisin';
import { AGENT_PATH } from '$lib/agent';
import { SHIFTS_SQL, STAFF_SQL, rowsToShifts, rowsToStaff, type SqlNodeRow } from '$lib/board-data';

/** agent_ref is a raisin reference object over HTTP; tolerate both shapes. */
function agentRefPath(item: ConversationListItem): string | undefined {
  const ref = item.agentRef as unknown;
  if (typeof ref === 'string') return ref;
  if (ref && typeof ref === 'object') return (ref as Record<string, string>)['raisin:path'];
  return undefined;
}

export const load: PageServerLoad = async ({ parent }) => {
  const { session } = await parent();
  if (!session) return { board: null, chat: null, tasks: null };

  const client = createHttpClient(session.token);
  const db = client.database(REPOSITORY);

  // Pending human tasks (inbox) load alongside the board so the task panel
  // is part of the server-rendered HTML, just like the shift cards.
  const [shiftsRes, staffRes, pendingTasks] = await Promise.all([
    db.executeSql(SHIFTS_SQL),
    db.executeSql(STAFF_SQL),
    listPendingTasks(client),
  ]);

  // Latest ai_chat conversation with our agent (same selection as the old SPA).
  let conversationPath: string | null = null;
  let messages: ChatMessage[] = [];
  try {
    const manager = createConversationManager(client);
    const conversations = await manager.list({ type: 'ai_chat' });
    conversationPath =
      conversations
        .filter((c) => agentRefPath(c) === AGENT_PATH)
        .sort((a, b) => (b.updatedAt ?? '').localeCompare(a.updatedAt ?? ''))[0]
        ?.conversationPath ?? null;
    if (conversationPath) {
      messages = await manager.getMessages(conversationPath);
    }
  } catch (err) {
    console.warn('[ssr] loading conversation failed, chat starts fresh:', err);
  }

  return {
    board: {
      shifts: rowsToShifts((shiftsRes.rows ?? []) as unknown as SqlNodeRow[]),
      staff: rowsToStaff((staffRes.rows ?? []) as unknown as SqlNodeRow[]),
    },
    chat: { conversationPath, messages },
    tasks: pendingTasks,
  };
};

export const actions: Actions = {
  login: async ({ request, cookies }) => {
    const form = await request.formData();
    const email = String(form.get('email') ?? '').trim();
    const password = String(form.get('password') ?? '');
    if (!email || !password) {
      return fail(400, { error: 'Email and password are required', email });
    }

    const result = await loginWithEmail(email, password);
    if (!result.ok) {
      return fail(401, { error: result.error, email });
    }

    setAuthCookies(cookies, result.tokens);
    // POST → redirect → GET: the follow-up GET runs hooks + load with the
    // fresh cookies, so the board is server-rendered right after login.
    redirect(303, '/');
  },

  logout: async ({ cookies }) => {
    clearAuthCookies(cookies);
    redirect(303, '/');
  },
};
