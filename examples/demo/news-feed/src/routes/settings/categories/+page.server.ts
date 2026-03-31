import { fail } from '@sveltejs/kit';
import { executeWithUser } from '$lib/server/db';
import { ARTICLES_PATH } from '$lib/types';

export async function load({ parent }) {
	const parentData = await parent();
	return {
		categories: parentData.categories
	};
}

export const actions = {
	create: async ({ request, locals }) => {
		const accessToken = locals.accessToken;
		if (!accessToken) {
			return fail(401, { error: 'Authentication required' });
		}

		const formData = await request.formData();
		const name = formData.get('name') as string;
		const slug = formData.get('slug') as string;
		const label = formData.get('label') as string;
		const color = formData.get('color') as string;

		if (!name || !slug || !label || !color) {
			return fail(400, { error: 'All fields are required' });
		}

		// Get current max order
		const path = `${ARTICLES_PATH}/${slug}`;

		try {
			// Create the category folder with properties
			await executeWithUser(`
				INSERT INTO social (path, node_type, name, properties)
				VALUES ($1, 'raisin:Folder', $2, $3::JSONB)
			`, [path, name, JSON.stringify({ label, color, order: 999 })], accessToken);

			return { success: true };
		} catch (err) {
			return fail(500, { error: (err as Error).message });
		}
	},

	update: async ({ request, locals }) => {
		const accessToken = locals.accessToken;
		if (!accessToken) {
			return fail(401, { error: 'Authentication required' });
		}

		const formData = await request.formData();
		const path = formData.get('path') as string;
		const name = formData.get('name') as string;
		const label = formData.get('label') as string;
		const color = formData.get('color') as string;

		if (!path || !name || !label || !color) {
			return fail(400, { error: 'All fields are required' });
		}

		try {
			await executeWithUser(`
				UPDATE social
				SET name = $1,
				    properties = properties || $2::JSONB
				WHERE path = $3
			`, [name, JSON.stringify({ label, color }), path], accessToken);

			return { success: true };
		} catch (err) {
			return fail(500, { error: (err as Error).message });
		}
	},

	delete: async ({ request, locals }) => {
		const accessToken = locals.accessToken;
		if (!accessToken) {
			return fail(401, { error: 'Authentication required' });
		}

		const formData = await request.formData();
		const path = formData.get('path') as string;

		if (!path) {
			return fail(400, { error: 'Path is required' });
		}

		try {
			// Delete the category folder (this should fail if it has children)
			await executeWithUser(`
				DELETE FROM social WHERE path = $1
			`, [path], accessToken);

			return { success: true };
		} catch (err) {
			return fail(500, { error: (err as Error).message });
		}
	},

	reorder: async ({ request, locals }) => {
		const accessToken = locals.accessToken;
		if (!accessToken) {
			return fail(401, { error: 'Authentication required' });
		}

		const formData = await request.formData();
		const sourcePath = formData.get('sourcePath') as string;
		const targetPath = formData.get('targetPath') as string;

		if (!sourcePath || !targetPath) {
			return fail(400, { error: 'Source and target paths are required' });
		}

		try {
			// Use ORDER to reposition the category
			console.log("Reordering:", sourcePath, "above", targetPath);
			await executeWithUser(`
				ORDER social SET path = $1 ABOVE path = $2
			`, [sourcePath, targetPath], accessToken);

			return { success: true };
		} catch (err) {
			return fail(500, { error: (err as Error).message });
		}
	}
};
