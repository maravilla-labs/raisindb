<script lang="ts">
	import { Stethoscope, LogOut } from 'lucide-svelte';
	import SidebarSection from './SidebarSection.svelte';
	import type { NavSection } from '$lib/config/customerNav';
	import type { User } from '$lib/stores/users';
	import { logout } from '$lib/stores/auth';
	import { goto } from '$app/navigation';

	interface Props {
		navigation: NavSection[];
		user: User | null;
		inboxCount?: number;
		queueCount?: number;
	}

	let { navigation, user, inboxCount = 0, queueCount = 0 }: Props = $props();

	function getBadgeCount(badge: string): number {
		if (badge === 'inbox') return inboxCount;
		if (badge === 'queue') return queueCount;
		return 0;
	}

	function handleLogout() {
		logout();
		goto('/login');
	}
</script>

<aside class="fixed left-0 top-0 z-40 flex h-screen w-64 flex-col border-r border-gray-200 bg-white">
	<!-- Logo -->
	<div class="flex h-16 items-center gap-3 border-b border-gray-200 px-4">
		<div class="flex h-10 w-10 items-center justify-center rounded-lg bg-blue-500">
			<Stethoscope class="h-6 w-6 text-white" />
		</div>
		<div>
			<div class="text-sm font-bold text-gray-900">Pocket Medico</div>
			<div class="text-xs text-gray-500">Medical Transcription</div>
		</div>
	</div>

	<!-- Navigation -->
	<nav class="flex-1 space-y-4 overflow-y-auto p-4">
		{#each navigation as section}
			<SidebarSection
				title={section.title}
				items={section.items}
				collapsible={section.collapsible}
				defaultExpanded={section.defaultExpanded}
				{getBadgeCount}
			/>
		{/each}
	</nav>

	<!-- User Footer -->
	{#if user}
		<div class="border-t border-gray-200 p-4">
			<div class="flex items-center gap-3">
				<div class="flex h-9 w-9 items-center justify-center rounded-full bg-gray-200 text-sm font-medium text-gray-600">
					{user.displayName.charAt(0).toUpperCase()}
				</div>
				<div class="flex-1 min-w-0">
					<div class="truncate text-sm font-medium text-gray-900">{user.displayName}</div>
					<div class="truncate text-xs text-gray-500">{user.email}</div>
				</div>
				<button
					onclick={handleLogout}
					class="rounded-lg p-2 text-gray-400 hover:bg-gray-100 hover:text-gray-600"
					title="Logout"
				>
					<LogOut class="h-4 w-4" />
				</button>
			</div>
		</div>
	{/if}
</aside>
