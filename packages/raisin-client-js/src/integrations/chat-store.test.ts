import { describe, expect, it } from 'vitest';
import type { ChatMessage } from '../types/chat';
import { projectPlansFromMessages } from '../utils/plan-projection';

describe('projectPlansFromMessages', () => {
  it('projects ai_plan and ai_task_update messages into deterministic plan state', () => {
    const messages: ChatMessage[] = [
      {
        role: 'assistant',
        content: 'Plan created',
        timestamp: '2026-02-14T10:00:00.000Z',
        messageType: 'ai_plan',
        data: {
          plan_id: 'plan-1',
          plan_path: '/agents/sample/inbox/chats/conv-1/reply-to-msg-1/plan-1',
          title: 'Weather Plan',
          status: 'pending_approval',
          requires_approval: true,
          tasks: [
            { task_id: 'task-1', title: 'Check Basel weather', status: 'pending' },
            { task_id: 'task-2', title: 'Check Bern weather', status: 'pending' },
          ],
        },
      },
      {
        role: 'assistant',
        content: 'Task updated',
        timestamp: '2026-02-14T10:01:00.000Z',
        messageType: 'ai_task_update',
        data: {
          plan_path: '/agents/sample/inbox/chats/conv-1/reply-to-msg-1/plan-1',
          task_id: 'task-1',
          task_title: 'Check Basel weather',
          status: 'completed',
          plan_status: 'in_progress',
        },
      },
    ];

    const plans = projectPlansFromMessages(messages);
    expect(plans).toHaveLength(1);

    const [plan] = plans;
    expect(plan.planId).toBe('plan-1');
    expect(plan.planPath).toBe('/agents/sample/inbox/chats/conv-1/reply-to-msg-1/plan-1');
    expect(plan.status).toBe('in_progress');
    expect(plan.requiresApproval).toBe(false);

    const task1 = plan.tasks.find((task) => task.taskId === 'task-1');
    expect(task1?.status).toBe('completed');
  });
});
