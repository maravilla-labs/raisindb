import type { Handle } from '@sveltejs/kit';
import { getSessionUser, getAccessToken } from '$lib/server/auth';

export const handle: Handle = async ({ event, resolve }) => {
	// Get user from session cookies
	event.locals.user = getSessionUser(event.cookies);

	// Store the access token for database queries with identity context
	event.locals.accessToken = getAccessToken(event.cookies);

	const response = await resolve(event);
	return response;
};
