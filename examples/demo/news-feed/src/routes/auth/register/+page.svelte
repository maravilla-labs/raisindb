<script lang="ts">
	import { enhance } from '$app/forms';
	import { Newspaper, Loader2, AlertCircle } from 'lucide-svelte';

	let { form } = $props();
	let loading = $state(false);
</script>

<svelte:head>
	<title>Create Account - News Feed</title>
</svelte:head>

<div class="flex min-h-[60vh] items-center justify-center">
	<div class="w-full max-w-md space-y-8 rounded-xl bg-white p-8 shadow-lg">
		<div class="text-center">
			<div class="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-blue-100">
				<Newspaper class="h-6 w-6 text-blue-600" />
			</div>
			<h2 class="mt-6 text-3xl font-bold text-gray-900">Create account</h2>
			<p class="mt-2 text-sm text-gray-600">
				Already have an account?
				<a href="/auth/login" class="font-medium text-blue-600 hover:text-blue-500">Sign in</a>
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
					<label for="display_name" class="block text-sm font-medium text-gray-700"
						>Display name</label
					>
					<input
						id="display_name"
						name="display_name"
						type="text"
						autocomplete="name"
						value={form?.displayName ?? ''}
						class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
						placeholder="John Doe"
					/>
				</div>

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
						autocomplete="new-password"
						required
						minlength="8"
						class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					/>
					<p class="mt-1 text-xs text-gray-500">Must be at least 8 characters</p>
				</div>

				<div>
					<label for="password_confirm" class="block text-sm font-medium text-gray-700"
						>Confirm password</label
					>
					<input
						id="password_confirm"
						name="password_confirm"
						type="password"
						autocomplete="new-password"
						required
						minlength="8"
						class="mt-1 block w-full rounded-lg border border-gray-300 px-3 py-2 shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
					/>
				</div>
			</div>

			<button
				type="submit"
				disabled={loading}
				class="flex w-full items-center justify-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
			>
				{#if loading}
					<Loader2 class="h-4 w-4 animate-spin" />
					Creating account...
				{:else}
					Create account
				{/if}
			</button>
		</form>
	</div>
</div>
