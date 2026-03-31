<script lang="ts">
	import '../../app.css';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { Sidebar, Toast } from '$lib/components/layout';
	import { customerNav } from '$lib/config/customerNav';
	import { currentUser, isCustomer, restoreSession } from '$lib/stores/auth';
	import { unreadCount } from '$lib/stores/inbox';

	let { children } = $props();
	let mounted = $state(false);

	onMount(() => {
		restoreSession();
		mounted = true;
	});

	// Auth guard
	$effect(() => {
		if (mounted && !$currentUser) {
			goto('/login');
		} else if (mounted && $currentUser && !$isCustomer) {
			goto('/nurse/dashboard'); // Redirect nurses to nurse dashboard
		}
	});
</script>

{#if $currentUser && $isCustomer}
	<div class="flex h-screen overflow-hidden bg-gray-50">
		<!-- Sidebar -->
		<Sidebar
			navigation={customerNav}
			user={$currentUser}
			inboxCount={$unreadCount}
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
