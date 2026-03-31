<script lang="ts">
	import { goto } from '$app/navigation';
	import { Button, Input } from '$lib/components/shared';
	import { login } from '$lib/stores/auth';
	import { findUserByEmail, registerUser, type UserRole } from '$lib/stores/users';
	import { toasts } from '$lib/stores/toast';
	import { User, Stethoscope } from 'lucide-svelte';

	let email = $state('');
	let password = $state('');
	let confirmPassword = $state('');
	let displayName = $state('');
	let practiceName = $state('');
	let role = $state<UserRole>('customer');
	let loading = $state(false);
	let error = $state('');

	async function handleSubmit(e: Event) {
		e.preventDefault();
		error = '';

		// Validation
		if (password !== confirmPassword) {
			error = 'Passwords do not match';
			return;
		}

		if (password.length < 6) {
			error = 'Password must be at least 6 characters';
			return;
		}

		// Check if email already exists
		if (findUserByEmail(email)) {
			error = 'An account with this email already exists';
			return;
		}

		loading = true;

		// Simulate network delay
		await new Promise((r) => setTimeout(r, 500));

		// Register user
		const newUser = registerUser({
			email,
			password,
			displayName,
			role,
			practiceName: role === 'customer' ? practiceName : undefined
		});

		login(newUser);
		toasts.success('Account created successfully!');

		// Redirect based on role
		if (role === 'nurse') {
			goto('/nurse/dashboard');
		} else {
			goto('/dashboard');
		}
	}
</script>

<svelte:head>
	<title>Register - Pocket Medico</title>
</svelte:head>

<form onsubmit={handleSubmit} class="space-y-6">
	<div class="text-center">
		<h1 class="text-2xl font-bold text-gray-900">Create account</h1>
		<p class="mt-1 text-sm text-gray-500">Get started with Pocket Medico</p>
	</div>

	{#if error}
		<div class="rounded-lg bg-red-50 p-3 text-sm text-red-700">
			{error}
		</div>
	{/if}

	<!-- Role Selection -->
	<div class="space-y-2">
		<label class="block text-sm font-medium text-gray-700">I am a...</label>
		<div class="grid grid-cols-2 gap-3">
			<button
				type="button"
				onclick={() => (role = 'customer')}
				class="flex flex-col items-center gap-2 rounded-lg border-2 p-4 transition-colors
					{role === 'customer'
					? 'border-blue-500 bg-blue-50 text-blue-700'
					: 'border-gray-200 text-gray-600 hover:border-gray-300'}"
			>
				<Stethoscope class="h-6 w-6" />
				<span class="text-sm font-medium">Doctor</span>
				<span class="text-xs text-gray-500">Order transcriptions</span>
			</button>

			<button
				type="button"
				onclick={() => (role = 'nurse')}
				class="flex flex-col items-center gap-2 rounded-lg border-2 p-4 transition-colors
					{role === 'nurse'
					? 'border-blue-500 bg-blue-50 text-blue-700'
					: 'border-gray-200 text-gray-600 hover:border-gray-300'}"
			>
				<User class="h-6 w-6" />
				<span class="text-sm font-medium">Nurse</span>
				<span class="text-xs text-gray-500">Review transcriptions</span>
			</button>
		</div>
	</div>

	<div class="space-y-4">
		<Input
			type="text"
			label="Full Name"
			placeholder="Enter your name"
			bind:value={displayName}
			required
		/>

		<Input
			type="email"
			label="Email"
			placeholder="Enter your email"
			bind:value={email}
			required
			autocomplete="email"
		/>

		{#if role === 'customer'}
			<Input
				type="text"
				label="Practice Name"
				placeholder="Enter your practice name"
				bind:value={practiceName}
				hint="Optional"
			/>
		{/if}

		<Input
			type="password"
			label="Password"
			placeholder="Create a password"
			bind:value={password}
			required
			autocomplete="new-password"
		/>

		<Input
			type="password"
			label="Confirm Password"
			placeholder="Confirm your password"
			bind:value={confirmPassword}
			required
			autocomplete="new-password"
		/>
	</div>

	<Button type="submit" class="w-full" {loading}>
		{loading ? 'Creating account...' : 'Create account'}
	</Button>

	<p class="text-center text-sm text-gray-500">
		Already have an account?
		<a href="/login" class="font-medium text-blue-600 hover:text-blue-500">
			Sign in
		</a>
	</p>
</form>
