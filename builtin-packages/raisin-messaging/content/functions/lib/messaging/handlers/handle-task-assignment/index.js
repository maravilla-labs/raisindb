/**
 * Handle Task Assignment — a person assigns a task through their outbox.
 *
 * The outbox message (`message_type: task_assignment`, `status: pending`) carries the task in its body:
 *
 *   {
 *     assignee: <user key | user id | home path | 'circle:<id>' | '/circles/<name>'>,
 *     task_type: 'sign_contract' | 'bring_item' | 'confirm_attendance' | any slug,
 *     title, description?, options?, input_schema?, data?, priority?, due_in_seconds?,
 *     conversation_id?,  // shows the task as a card in that thread (a circle defaults to its group chat)
 *     attachments?,      // asset references below assets:/messages/<conversation id>/
 *     completion?: 'any' | 'all' | 'quorum', required?, response_condition?   // circles only
 *   }
 *
 * The sender is WHERE the message lives (its outbox), never a field. A person-to-person task is held to
 * `task_permissions` of /config/messaging (blocks included); a task for a whole circle to the circle's
 * `messaging.tasks` policy (moderators by default). The task itself is created by `raisin.tasks.create`,
 * which for a circle writes one task into every member's inbox and the system-only record that holds the
 * completion policy (docs/plans/group-messaging-and-tasks.md). The assignment id is derived from the
 * message, so a retried trigger assigns once.
 *
 * Outcome on the outbox node: `status: delivered` with `task_path` or `assignment_id`, or `status: failed`
 * with `failure_reason` (bad_outbox_path, sender_mismatch, bad_task, unknown_assignee, not_allowed,
 * unknown_circle, not_a_member, tasks_closed, group_too_large, bad_attachments, too_many_attachments,
 * foreign_attachment, assignment_failed).
 *
 * Replaces the Starlark handler, which took the sender from `sender_path` in the message, checked no
 * permission and knew only `/users/<slug>` homes.
 */

import {
  entityPathFromOutboxPath, resolveSenderHome, claimsMatch, refuse, resolveUser, ensureFolderExists,
  createIdempotent, sqlRows,
} from '../shared/parties.js';
import { checkPermission, checkAttachments } from '../shared/rules.js';
import { circleReference, resolveCircle, rosterFor, mayAct, conversationIdOf, membersOf } from '../shared/circles.js';

const AC = 'raisin:access_control';
const TAG = 'handle-task-assignment';
const TASK_TYPE = /^[a-z][a-z0-9_-]{0,63}$/;
const CONVERSATION = /^[A-Za-z0-9_-]{1,128}$/;

export async function handle_task_assignment(input) {
  const flowInput = input.flow_input ?? input;
  const node = flowInput.node;
  const workspace = flowInput.workspace ?? AC;
  if (!node) throw new Error('No message node provided');

  const props = node.properties ?? {};
  if (props.message_type !== 'task_assignment') throw new Error('Not a task assignment message');
  const nodePath = node.path ?? '';
  const body = props.body && typeof props.body === 'object' ? props.body : {};

  // 1. The sender is the home whose outbox holds the message.
  const outboxHome = entityPathFromOutboxPath(nodePath);
  if (!outboxHome || workspace !== AC) return refuse(workspace, nodePath, 'bad_outbox_path', TAG);
  const sender = await resolveSenderHome(workspace, outboxHome);
  if (!sender || sender.isAgent || sender.isVisitor) return refuse(workspace, nodePath, 'bad_outbox_path', TAG);
  if (!claimsMatch(sender, outboxHome, {
    ids: [props.sender_id, body.sender_id],
    paths: [props.sender_path, body.sender_path],
    emails: [props.sender_email, body.sender_email],
    keys: [props.sender_key, body.sender_key],
  })) {
    return refuse(workspace, nodePath, 'sender_mismatch', TAG);
  }

  // 2. The task.
  const taskType = body.task_type ?? 'action';
  const title = typeof body.title === 'string' && body.title.trim() ? body.title.trim() : props.subject;
  if (!TASK_TYPE.test(String(taskType)) || typeof title !== 'string' || !title.trim() || title.length > 300) {
    return refuse(workspace, nodePath, 'bad_task', TAG);
  }
  const assignedBy = { key: sender.key ?? null, display_name: sender.displayName };
  const task = {
    task_type: taskType,
    title: title.trim(),
    assigned_by: assignedBy,
    source_message_id: String(node.id),
  };
  if (typeof body.description === 'string') task.description = body.description.slice(0, 5000);
  if (Array.isArray(body.options)) task.options = body.options.slice(0, 20);
  if (body.input_schema && typeof body.input_schema === 'object') task.input_schema = body.input_schema;
  if (body.data && typeof body.data === 'object' && !Array.isArray(body.data)) task.data = body.data;
  const priority = Number(body.priority);
  if (Number.isInteger(priority) && priority >= 1 && priority <= 5) task.priority = priority;
  const due = Number(body.due_in_seconds);
  if (Number.isFinite(due) && due > 0) task.due_in_seconds = Math.floor(due);

  // 3. A whole circle.
  const circleRef = circleReference({}, { assignee: body.assignee, circle: body.circle });
  if (circleRef) return assignToCircle({ workspace, node, body, sender, circleRef, task });

  // 4. One person.
  const recipient = await resolveAssignee(body);
  if (!recipient) return refuse(workspace, nodePath, 'unknown_assignee', TAG);
  const perm = await checkPermission(sender.id, recipient.id, 'task_assignment');
  if (!perm.allowed) return refuse(workspace, nodePath, 'not_allowed', TAG);

  if (body.conversation_id !== undefined) {
    if (typeof body.conversation_id !== 'string' || !CONVERSATION.test(body.conversation_id)) {
      return refuse(workspace, nodePath, 'bad_task', TAG);
    }
    task.conversation_id = body.conversation_id;
  }
  const attachments = checkAttachments(body.attachments, task.conversation_id);
  if (attachments.error) return refuse(workspace, nodePath, attachments.error, TAG);
  if (attachments.list.length) task.attachments = attachments.list;

  await copyToSent(workspace, node, outboxHome);

  // A retried trigger finds the task it already created.
  let taskPath = await existingTaskFor(recipient.path, node.id);
  if (!taskPath) {
    const created = await raisin.tasks.create({ ...task, assignee: recipient.path });
    taskPath = created?.task_path ?? null;
    if (taskPath) await notify(recipient, task, taskPath, node.id);
  }

  await raisin.nodes.updateProperty(workspace, nodePath, 'status', 'delivered');
  await raisin.nodes.updateProperty(workspace, nodePath, 'delivered_at', new Date().toISOString());
  return { success: true, task_path: taskPath };
}

async function assignToCircle({ workspace, node, body, sender, circleRef, task }) {
  const nodePath = node.path;
  const circle = await resolveCircle(circleRef);
  if (!circle) return refuse(workspace, nodePath, 'unknown_circle', TAG);
  if (circle.policy.tasks === 'off') return refuse(workspace, nodePath, 'tasks_closed', TAG);
  const roster = await rosterFor(circle, sender.id);
  if (!roster.role) return refuse(workspace, nodePath, 'not_a_member', TAG);
  if (!mayAct(circle.policy, roster.role, 'tasks')) return refuse(workspace, nodePath, 'not_allowed', TAG);
  if (roster.memberCount > circle.policy.max_members) return refuse(workspace, nodePath, 'group_too_large', TAG);
  if (roster.memberCount === 0) return refuse(workspace, nodePath, 'not_allowed', TAG);

  const conversationId = conversationIdOf(circle);
  const attachments = checkAttachments(body.attachments, conversationId);
  if (attachments.error) return refuse(workspace, nodePath, attachments.error, TAG);

  await copyToSent(workspace, node, entityPathFromOutboxPath(nodePath));

  const request = {
    ...task,
    assignee: circle.path,
    assignment_id: assignmentIdFor(node.id),
    conversation_id: conversationId,
    completion: ['any', 'all', 'quorum'].includes(body.completion) ? body.completion : 'any',
  };
  if (attachments.list.length) request.attachments = attachments.list;
  const required = Number(body.required);
  if (Number.isInteger(required) && required > 0) request.required = required;
  if (typeof body.response_condition === 'string' && body.response_condition.length <= 500) {
    request.response_condition = body.response_condition;
  }

  let created;
  try {
    created = await raisin.tasks.create(request);
  } catch (e) {
    console.warn(`[${TAG}] ${nodePath}: ${e?.message || e}`);
    return refuse(workspace, nodePath, 'assignment_failed', TAG);
  }
  if (created?.created !== false) {
    const members = await membersOf(circle);
    const byHome = new Map(members.map((m) => [m.path, m]));
    for (const taskPath of created?.task_paths ?? []) {
      const member = byHome.get(taskPath.split('/inbox/')[0]);
      if (member) await notify(member, task, taskPath, node.id, circle.name);
    }
  }

  await raisin.nodes.updateProperty(workspace, nodePath, 'status', 'delivered');
  await raisin.nodes.updateProperty(workspace, nodePath, 'delivered_at', new Date().toISOString());
  await raisin.nodes.updateProperty(workspace, nodePath, 'assignment_id', created?.assignment_id ?? request.assignment_id);
  return { success: true, assignment_id: created?.assignment_id ?? request.assignment_id };
}

/** A stable assignment id for one outbox message: a retried trigger finds the record it wrote. */
export function assignmentIdFor(messageId) {
  return `ga-${String(messageId).replace(/[^A-Za-z0-9_-]/g, '').slice(0, 80)}`;
}

async function resolveAssignee(body) {
  const a = body.assignee ?? body.assignee_id ?? body.assignee_path ?? body.assignee_key;
  if (typeof a !== 'string' || !a) return null;
  return resolveUser(AC, a.startsWith('/') ? a : null, a.startsWith('/') ? null : a, null, null);
}

async function existingTaskFor(home, messageId) {
  const rows = await sqlRows(
    `SELECT path FROM '${AC}' WHERE CHILD_OF($1) AND node_type = 'raisin:InboxTask'
       AND properties->>'source_message_id' = $2 LIMIT 1`,
    [`${home}/inbox`, String(messageId)],
  );
  return rows[0]?.path ?? null;
}

async function copyToSent(workspace, node, home) {
  if (!home) return;
  const sentPath = await ensureFolderExists(workspace, home, 'sent', 'Sent');
  const slug = node.path.split('/').pop();
  const sentProps = { ...(node.properties ?? {}), status: 'sent', sent_at: new Date().toISOString() };
  delete sentProps.sender_key;
  await createIdempotent(workspace, sentPath, {
    slug, name: slug, node_type: node.node_type ?? 'raisin:Message', properties: sentProps,
  });
}

async function notify(person, task, taskPath, messageId, groupName = null) {
  try {
    const folder = await ensureFolderExists(AC, `${person.path}/inbox`, 'notifications', 'Notifications');
    const slug = `notif-task-${String(messageId).slice(-8)}`;
    await createIdempotent(AC, folder, {
      slug, name: slug, node_type: 'raisin:Notification',
      properties: {
        type: 'task_assignment',
        title: groupName ? `${groupName}: ${task.title}` : `New task: ${task.title}`,
        body: task.description ? String(task.description).slice(0, 120) : '',
        link: taskPath.slice(person.path.length),
        read: false,
        priority: task.priority ?? 3,
        data: {
          task_type: task.task_type,
          assigned_by: task.assigned_by,
          conversation_id: task.conversation_id ?? null,
        },
      },
    });
  } catch (e) {
    console.warn(`[${TAG}] notification failed: ${e?.message || e}`);
  }
}
