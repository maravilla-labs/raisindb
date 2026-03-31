<script lang="ts">
	import '../../app.css';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { Sidebar, Toast } from '$lib/components/layout';
	import { nurseNav } from '$lib/config/nurseNav';
	import { currentUser, isNurse, restoreSession } from '$lib/stores/auth';
	import { unreadCount } from '$lib/stores/inbox';
	import { nurseQueue } from '$lib/stores/orders';

	let { children } = $props();
	let mounted = $state(false);

	// Derived queue count for sidebar badge
	let queueCount = $derived($nurseQueue.length);

	onMount(() => {
		restoreSession();
		mounted = true;
	});

	// Auth guard - redirect to /nurse/dashboard or /dashboard based on role
	$effect(() => {
		if (mounted && !$currentUser) {
			goto('/login');
		} else if (mounted && $currentUser && !$isNurse) {
			goto('/dashboard'); // Redirect customers to customer dashboard
		}
	});
</script>

{#if $currentUser && $isNurse}
	<div class="flex h-screen overflow-hidden bg-gray-50">
		<!-- Sidebar -->
		<Sidebar
			navigation={nurseNav}
			user={$currentUser}
			inboxCount={$unreadCount}
			{queueCount}
		/>

		<!-- Main Content -->
		<div class="ml-64 flex flex-1 flex-col overflow-hidden">
			<main class="flex-1 overflow-y-auto">
				{@render children()}
			</main>
		</div>
	</div>

	<Toast />
{:else}
	<div class="flex h-screen items-center justify-center bg-gray-50">
		<div class="h-8 w-8 animate-spin rounded-full border-4 border-blue-500 border-t-transparent"></div>
	</div>
{/if}
