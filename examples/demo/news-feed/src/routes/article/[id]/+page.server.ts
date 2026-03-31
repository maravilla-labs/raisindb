import { error } from '@sveltejs/kit';
import { query, queryOne, execute } from '$lib/server/db';
import type { Article } from '$lib/types';

export async function load({ params }) {
	const article = await queryOne<Article>(`
		SELECT id, path, name, node_type, properties, created_at, updated_at
		FROM social
		WHERE id = $1
		  AND node_type = 'news:Article'
	`, [params.id]);

	if (!article) {
		throw error(404, 'Article not found');
	}

	// Increment view count
	await execute(`
		UPDATE social
		SET properties = jsonb_set(
			properties,
			'{views}',
			COALESCE((properties ->> 'views')::int, 0) + 1
		)
		WHERE id = $1
	`, [params.id]);
	// Get related articles (same category)
	const related = await query<Article>(`
		SELECT id, path, name, node_type, properties, created_at, updated_at
		FROM social
		WHERE node_type = 'news:Article'
		  AND PATH_STARTS_WITH(path, '/superbigshit/')
		  AND properties ->> 'category'::TEXT = $1
		  AND properties ->> 'status'::TEXT = 'published'
		  AND path != $2
		ORDER BY created_at DESC
		LIMIT 5
	`, [article.properties.category, article.path]);

	return {
		article,
		related
	};
}
