import { json } from '@sveltejs/kit';
import { executeWithUser } from '$lib/server/db';
import { ARTICLES_PATH } from '$lib/types';

export async function DELETE({ params, locals }) {
	const fullDbPath = `${ARTICLES_PATH}/${params.path}`;
	const accessToken = locals.accessToken;

	// Require authentication for delete operations
	if (!accessToken) {
		return json({ error: 'Authentication required to delete articles' }, { status: 401 });
	}

	try {
		const rowCount = await executeWithUser(`
			DELETE FROM social WHERE path = $1
		`, [fullDbPath], accessToken);

		if (rowCount === 0) {
			return json({ error: 'Article not found' }, { status: 404 });
		}

		return json({ success: true });
	} catch (err) {
		return json({ error: (err as Error).message }, { status: 500 });
	}
}
