import { redirect } from '@sveltejs/kit';
import { clearAuthCookies } from '$lib/server/auth';
import type { Actions, PageServerLoad } from './$types';

export const load: PageServerLoad = async () => {
	// Redirect to home - logout should be a POST action
	redirect(303, '/');
};

export const actions: Actions = {
	default: async ({ cookies }) => {
		clearAuthCookies(cookies);
		redirect(303, '/');
	}
};
