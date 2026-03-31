import { error, redirect, fail } from '@sveltejs/kit';
import { queryOne, executeWithUser } from '$lib/server/db';
import { ARTICLES_PATH, type Article, getCategoryFromPath, getSlugFromPath } from '$lib/types';

export async function load({ params, parent }) {
	const fullDbPath = `${ARTICLES_PATH}/${params.path}`;
	const parentData = await parent();

	const article = await queryOne<Article>(`
		SELECT id, path, name, node_type, properties, created_at, updated_at
		FROM social
		WHERE path = $1
		  AND node_type = 'news:Article'
	`, [fullDbPath]);

	if (!article) {
		throw error(404, 'Article not found');
	}

	const currentCategory = getCategoryFromPath(article.path);

	return {
		article,
		currentCategory,
		categories: parentData.categories
	};
}

export const actions = {
	default: async ({ request, params, locals }) => {
		const formData = await request.formData();
		const originalPath = `${ARTICLES_PATH}/${params.path}`;
		const newCategory = formData.get('category') as string;
		const accessToken = locals.accessToken;

		// Require authentication for write operations
		if (!accessToken) {
			return fail(401, { error: 'Authentication required to move articles' });
		}

		if (!newCategory) {
			return fail(400, { error: 'Category is required' });
		}

		// Get the current article slug
		const slug = getSlugFromPath(originalPath);
		const currentCategory = getCategoryFromPath(originalPath);

		// If category hasn't changed, just redirect back
		if (newCategory === currentCategory) {
			throw redirect(303, `/articles/${currentCategory}/${slug}`);
		}

		try {
			// Use MOVE to relocate the article to the new category
			const newParentPath = `${ARTICLES_PATH}/${newCategory}`;

			await executeWithUser(`
				MOVE social SET path = $1 TO path = $2
			`, [originalPath, newParentPath], accessToken);

			throw redirect(303, `/articles/${newCategory}/${slug}`);
		} catch (err) {
			if ((err as { status?: number }).status === 303) {
				throw err;
			}
			return fail(500, { error: (err as Error).message });
		}
	}
};
