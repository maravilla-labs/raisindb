import { fail, redirect } from '@sveltejs/kit';
import { login, setAuthCookies } from '$lib/server/auth';
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
		const email = data.get('email')?.toString() ?? '';
		const password = data.get('password')?.toString() ?? '';
		const rememberMe = data.get('remember_me') === 'on';

		if (!email || !password) {
			return fail(400, {
				error: 'Email and password are required',
				email
			});
		}

		const result = await login(email, password, rememberMe);

		if (!result.success) {
			return fail(401, {
				error: result.error.message,
				email
			});
		}

		// Set auth cookies
		setAuthCookies(cookies, result.tokens);

		// Redirect to home page
		redirect(303, '/');
	}
};
