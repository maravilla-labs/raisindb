<script lang="ts">
  import { user } from '$lib/stores/auth';
  import { inbox, outbox, sent, messagesLoading } from '$lib/stores/messages';
  import { Mail, Inbox as InboxIcon, Send, CheckCircle, Clock, AlertCircle } from 'lucide-svelte';

  const stats = $derived([
    { label: 'Inbox', value: $inbox.length, icon: InboxIcon, color: 'text-blue-600' },
    { label: 'Outbox (Pending)', value: $outbox.length, icon: Clock, color: 'text-yellow-600' },
    { label: 'Sent', value: $sent.length, icon: Send, color: 'text-green-600' },
  ]);
</script>

<div class="dashboard">
  <div class="header">
    <h1>Messaging System Test</h1>
    <p>Test the messaging implementation using the phase1test workspace</p>
  </div>

  {#if $user}
    <div class="user-card">
      <h2>Logged in as</h2>
      <div class="user-details">
        <div class="detail">
          <span class="label">User ID:</span>
          <span class="value">{$user.id}</span>
        </div>
        <div class="detail">
          <span class="label">Display Name:</span>
          <span class="value">{$user.displayName || 'Not set'}</span>
        </div>
      </div>
    </div>

    <div class="stats-grid">
      {#each stats as stat}
        <div class="stat-card">
          <div class="stat-icon {stat.color}">
            <svelte:component this={stat.icon} size={24} />
          </div>
          <div class="stat-content">
            <div class="stat-value">{stat.value}</div>
            <div class="stat-label">{stat.label}</div>
          </div>
        </div>
      {/each}
    </div>

    {#if $messagesLoading}
      <div class="loading">Loading messages...</div>
    {/if}

    <div class="quick-links">
      <h2>Quick Actions</h2>
      <div class="links-grid">
        <a href="/send" class="link-card">
          <Send size={32} />
          <span>Send Messages</span>
          <p>Create and send messages of different types</p>
        </a>
        <a href="/inbox" class="link-card">
          <InboxIcon size={32} />
          <span>View Inbox</span>
          <p>Check received messages and notifications</p>
        </a>
        <a href="/users" class="link-card">
          <Mail size={32} />
          <span>Manage Users</span>
          <p>View all users and their IDs</p>
        </a>
      </div>
    </div>
  {:else}
    <div class="guest-message">
      <AlertCircle size={48} />
      <h2>Not Logged In</h2>
      <p>Please <a href="/auth">login or register</a> to test the messaging system.</p>
    </div>
  {/if}

  <div class="test-guide">
    <h2>Testing Guide</h2>
    <div class="guide-content">
      <h3>Setup</h3>
      <ol>
        <li>Start RaisinDB server with phase1test repository</li>
        <li>Register 2-3 test users (e.g., alice, bob, charlie)</li>
        <li>Each user will have a global user ID (UUID)</li>
      </ol>

      <h3>Message Types to Test</h3>
      <table>
        <thead>
          <tr>
            <th>Type</th>
            <th>Description</th>
            <th>Expected Result</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><code>system_notification</code></td>
            <td>Simple notification</td>
            <td>Notification in recipient's inbox/notifications</td>
          </tr>
          <tr>
            <td><code>task_assignment</code></td>
            <td>Assign a task</td>
            <td>InboxTask + Notification in recipient's inbox</td>
          </tr>
          <tr>
            <td><code>relationship_request</code></td>
            <td>Request relationship (e.g., MANAGER_OF)</td>
            <td>Message in recipient's inbox for approval</td>
          </tr>
          <tr>
            <td><code>relationship_response</code></td>
            <td>Accept/reject request</td>
            <td>Graph edge created if accepted</td>
          </tr>
          <tr>
            <td><code>ward_invitation</code></td>
            <td>Create ward account</td>
            <td>New internal user + PARENT_OF edge</td>
          </tr>
          <tr>
            <td><code>stewardship_request</code></td>
            <td>Request stewardship</td>
            <td>Request in target's inbox</td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</div>

<style>
  .dashboard {
    max-width: 1200px;
    margin: 0 auto;
  }

  .header {
    margin-bottom: 2rem;
  }

  .header h1 {
    font-size: 2rem;
    font-weight: 700;
    color: #111827;
    margin: 0 0 0.5rem 0;
  }

  .header p {
    color: #6b7280;
    margin: 0;
  }

  .user-card {
    background: white;
    border-radius: 0.75rem;
    padding: 1.5rem;
    margin-bottom: 1.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .user-card h2 {
    font-size: 1rem;
    font-weight: 600;
    color: #6b7280;
    margin: 0 0 1rem 0;
  }

  .user-details {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .detail {
    display: flex;
    gap: 0.5rem;
  }

  .detail .label {
    font-weight: 500;
    color: #374151;
    min-width: 120px;
  }

  .detail .value {
    color: #6b7280;
  }

  .detail code {
    font-family: monospace;
    background: #f3f4f6;
    padding: 0.125rem 0.5rem;
    border-radius: 0.25rem;
    font-size: 0.875rem;
  }

  .stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 1rem;
    margin-bottom: 2rem;
  }

  .stat-card {
    background: white;
    border-radius: 0.75rem;
    padding: 1.5rem;
    display: flex;
    align-items: center;
    gap: 1rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .stat-icon {
    padding: 0.75rem;
    background: #f3f4f6;
    border-radius: 0.5rem;
  }

  .stat-value {
    font-size: 1.5rem;
    font-weight: 700;
    color: #111827;
  }

  .stat-label {
    font-size: 0.875rem;
    color: #6b7280;
  }

  .loading {
    text-align: center;
    padding: 1rem;
    color: #6b7280;
  }

  .quick-links {
    margin-bottom: 2rem;
  }

  .quick-links h2 {
    font-size: 1.25rem;
    font-weight: 600;
    margin: 0 0 1rem 0;
  }

  .links-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
    gap: 1rem;
  }

  .link-card {
    background: white;
    border-radius: 0.75rem;
    padding: 1.5rem;
    text-decoration: none;
    color: inherit;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    transition: all 0.2s;
  }

  .link-card:hover {
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);
  }

  .link-card :global(svg) {
    color: #6366f1;
  }

  .link-card span {
    font-size: 1.125rem;
    font-weight: 600;
    color: #111827;
  }

  .link-card p {
    font-size: 0.875rem;
    color: #6b7280;
    margin: 0;
  }

  .guest-message {
    background: white;
    border-radius: 0.75rem;
    padding: 3rem;
    text-align: center;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
    margin-bottom: 2rem;
  }

  .guest-message :global(svg) {
    color: #9ca3af;
    margin-bottom: 1rem;
  }

  .guest-message h2 {
    margin: 0 0 0.5rem 0;
    color: #374151;
  }

  .guest-message p {
    color: #6b7280;
    margin: 0;
  }

  .guest-message a {
    color: #6366f1;
    font-weight: 500;
  }

  .test-guide {
    background: white;
    border-radius: 0.75rem;
    padding: 1.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
  }

  .test-guide h2 {
    font-size: 1.25rem;
    font-weight: 600;
    margin: 0 0 1rem 0;
    color: #111827;
  }

  .guide-content h3 {
    font-size: 1rem;
    font-weight: 600;
    color: #374151;
    margin: 1.5rem 0 0.75rem 0;
  }

  .guide-content h3:first-child {
    margin-top: 0;
  }

  .guide-content ol {
    margin: 0;
    padding-left: 1.5rem;
    color: #4b5563;
  }

  .guide-content li {
    margin-bottom: 0.5rem;
  }

  .guide-content code {
    font-family: monospace;
    background: #f3f4f6;
    padding: 0.125rem 0.5rem;
    border-radius: 0.25rem;
    font-size: 0.875rem;
  }

  .guide-content table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 0.5rem;
  }

  .guide-content th,
  .guide-content td {
    text-align: left;
    padding: 0.75rem;
    border-bottom: 1px solid #e5e7eb;
  }

  .guide-content th {
    font-weight: 600;
    color: #374151;
    background: #f9fafb;
  }

  .guide-content td {
    color: #4b5563;
  }

  .text-blue-600 {
    color: #2563eb;
  }

  .text-yellow-600 {
    color: #ca8a04;
  }

  .text-green-600 {
    color: #16a34a;
  }
</style>
