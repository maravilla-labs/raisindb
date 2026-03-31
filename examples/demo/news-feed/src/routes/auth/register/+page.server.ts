import { fail, redirect } from '@sveltejs/kit';
import { register, setAuthCookies } from '$lib/server/auth';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async ({ locals }) => {
	// Redirect if already logged in
	if (locals.user) {
		redirect(303, '/');
	}
};

export const actions: Actions = {
	default: async ({ request, cookies }) => {
		const data = await request.formData();
		const displayName = data.get('display_name')?.toString() ?? '';
		const email = data.get('email')?.toString() ?? '';
		const password = data.get('password')?.toString() ?? '';
		const passwordConfirm = data.get('password_confirm')?.toString() ?? '';

		if (!email || !password) {
			return fail(400, {
				error: 'Email and password are required',
				email,
				displayName
			});
		}

		if (password !== passwordConfirm) {
			return fail(400, {
				error: 'Passwords do not match',
				email,
				displayName
			});
		}

		if (password.length < 8) {
			return fail(400, {
				error: 'Password must be at least 8 characters',
				email,
				displayName
			});
		}

		const result = await register(email, password, displayName || undefined);

		if (!result.success) {
			return fail(400, {
				error: result.error.message,
				email,
				displayName
			});
		}

		// Set auth cookies
		setAuthCookies(cookies, result.tokens);

		// Redirect to home page
		redirect(303, '/');
	}
};
