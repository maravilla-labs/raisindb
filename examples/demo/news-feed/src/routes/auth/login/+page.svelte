<script lang="ts">
	import { enhance } from '$app/forms';
	import { Newspaper, Loader2, AlertCircle } from 'lucide-svelte';

	let { form } = $props();
	let loading = $state(false);
</script>

<svelte:head>
	<title>Login - News Feed</title>
</svelte:head>

<div class="flex min-h-[60vh] items-center justify-center">
	<div class="w-full max-w-md space-y-8 rounded-xl bg-white p-8 shadow-lg">
		<div class="text-center">
			<div class="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-blue-100">
				<Newspaper class="h-6 w-6 text-blue-600" />
			</div>
			<h2 class="mt-6 text-3xl font-bold text-gray-900">Sign in</h2>
			<p class="mt-2 text-sm text-gray-600">
				Don't have an account?
				<a href="/auth/register" class="font-medium text-blue-600 hover:text-blue-500">
					Create one
				</a>
			</p>
		</div>

		{#if form?.error}
			<div class="flex items-center gap-2 rounded-lg bg-red-50 p-4 text-red-700">
				<AlertCircle class="h-5 w-5 flex-shrink-0" />
				<span class="text-sm">{form.error}</span>
			</div>
		{/if}

		<form
			method="POST"
			class="mt-8 space-y-6"
			use:enhance={() => {
				loading = true;
				return async ({ update }) => {
					loading = false;
					await update();
				};
			}}
		>
			<div class="space-y-4">
				<div>
					<label for="email" class="block text-sm font-medium text-gray-700">Email address</label>
					<input
						id="email"
						name="email"
						type="email"
						autocomplete="email"
						required
						value={form?.email ?? ''}
						class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					/>
				</div>

				<div>
					<label for="password" class="block text-sm font-medium text-gray-700">Password</label>
					<input
						id="password"
						name="password"
						type="password"
						autocomplete="current-password"
						required
						class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					/>
				</div>

				<div class="flex items-center">
					<input
						id="remember_me"
						name="remember_me"
						type="checkbox"
						class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
					/>
					<label for="remember_me" class="ml-2 block text-sm text-gray-700">Remember me</label>
				</div>
			</div>

			<button
				type="submit"
				disabled={loading}
				class="flex w-full items-center justify-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
			>
				{#if loading}
					<Loader2 class="h-4 w-4 animate-spin" />
					Signing in...
				{:else}
					Sign in
				{/if}
			</button>
		</form>
	</div>
</div>
