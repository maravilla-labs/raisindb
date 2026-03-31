import { queryMaybeUser } from '$lib/server/db';
import { ARTICLES_PATH, type Article } from '$lib/types';

export async function load({ parent, locals }) {
	// Get categories from parent layout
	const parentData = await parent();
	const accessToken = locals.accessToken;

	// Featured articles: published, featured, and publishing_date <= now
	const featured = await queryMaybeUser<Article>(`
		SELECT id, path, name, node_type, properties, created_at, updated_at
		FROM social
		WHERE DESCENDANT_OF('${ARTICLES_PATH}')
		  AND node_type = 'news:Article'
		  AND properties @> '{"featured": true, "status": "published"}'
		  AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
		ORDER BY properties ->> 'publishing_date' DESC
		LIMIT 3
	`, [], accessToken);

	// Recent articles: published and publishing_date <= now
	const recent = await queryMaybeUser<Article>(`
		SELECT id, path, name, node_type, properties, created_at, updated_at
		FROM social
		WHERE DESCENDANT_OF('${ARTICLES_PATH}')
		  AND node_type = 'news:Article'
		  AND properties ->> 'status'::TEXT = 'published'
		  AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
		ORDER BY properties ->> 'publishing_date' DESC
		LIMIT 12
	`, [], accessToken);

	return {
		featured,
		recent,
		categories: parentData.categories
	};
}
