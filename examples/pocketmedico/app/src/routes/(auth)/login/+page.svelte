<script lang="ts">
	import { goto } from '$app/navigation';
	import { Button, Input } from '$lib/components/shared';
	import { login } from '$lib/stores/auth';
	import { validateCredentials } from '$lib/stores/users';
	import { toasts } from '$lib/stores/toast';

	let email = $state('');
	let password = $state('');
	let loading = $state(false);
	let error = $state('');

	async function handleSubmit(e: Event) {
		e.preventDefault();
		error = '';
		loading = true;

		// Simulate network delay
		await new Promise((r) => setTimeout(r, 500));

		const user = validateCredentials(email, password);

		if (user) {
			login(user);
			toasts.success(`Welcome back, ${user.displayName}!`);

			// Redirect based on role
			if (user.role === 'nurse') {
				goto('/nurse/dashboard');
			} else {
				goto('/dashboard');
			}
		} else {
			error = 'Invalid email or password';
			loading = false;
		}
	}
</script>

<svelte:head>
	<title>Login - Pocket Medico</title>
</svelte:head>

<form onsubmit={handleSubmit} class="space-y-6">
	<div class="text-center">
		<h1 class="text-2xl font-bold text-gray-900">Welcome back</h1>
		<p class="mt-1 text-sm text-gray-500">Sign in to your account</p>
	</div>

	{#if error}
		<div class="rounded-lg bg-red-50 p-3 text-sm text-red-700">
			{error}
		</div>
	{/if}

	<div class="space-y-4">
		<Input
			type="email"
			label="Email"
			placeholder="Enter your email"
			bind:value={email}
			required
			autocomplete="email"
		/>

		<Input
			type="password"
			label="Password"
			placeholder="Enter your password"
			bind:value={password}
			required
			autocomplete="current-password"
		/>
	</div>

	<Button type="submit" class="w-full" {loading}>
		{loading ? 'Signing in...' : 'Sign in'}
	</Button>

	<p class="text-center text-sm text-gray-500">
		Don't have an account?
		<a href="/register" class="font-medium text-blue-600 hover:text-blue-500">
			Register
		</a>
	</p>
</form>
