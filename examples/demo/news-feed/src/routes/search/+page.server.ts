import { query } from '$lib/server/db';
import { ARTICLES_PATH, TAGS_PATH, type Article } from '$lib/types';

// Workspace name for REFERENCES predicate
const WORKSPACE = 'social';

export async function load({ url, parent }) {
	const parentData = await parent();
	const q = url.searchParams.get('q') || '';
	const tagPath = url.searchParams.get('tag') || '';

	if (!q.trim() && !tagPath.trim()) {
		return { query: '', tag: '', articles: [], categories: parentData.categories, tags: parentData.tags };
	}

	let articles: Article[];

	if (tagPath.trim()) {
		// Filter by tag using REFERENCES predicate
		// Uses the reverse reference index for efficient O(k) lookup
		// Format: REFERENCES('workspace:/path')

		// Build the REFERENCES target: 'social:/superbigshit/tags/tech-stack/rust'
		const referencesTarget = `${WORKSPACE}:${tagPath.trim()}`;

		// Use REFERENCES predicate for efficient index-based lookup
		// This leverages the ref_rev column family in RocksDB
		// Only show published articles with publishing_date <= now
		articles = await query<Article>(`
			SELECT id, path, name, node_type, properties, created_at, updated_at
			FROM social
			WHERE REFERENCES('${referencesTarget}')
			  AND node_type = 'news:Article'
			  AND properties ->> 'status'::TEXT = 'published'
			  AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
			ORDER BY properties ->> 'publishing_date' DESC
			LIMIT 20
		`);
	} else {
		// Keyword search using ILIKE on title, body, excerpt, and keywords
		// Only show published articles with publishing_date <= now
		articles = await query<Article>(`
			SELECT id, path, name, node_type, properties, created_at, updated_at
			FROM social
			WHERE DESCENDANT_OF('${ARTICLES_PATH}')
			  AND node_type = 'news:Article'
			  AND properties ->> 'status'::TEXT = 'published'
			  AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
			  AND (
			    COALESCE(properties ->> 'title', '') ILIKE '%' || $1 || '%'
			    OR COALESCE(properties ->> 'body', '') ILIKE '%' || $1 || '%'
			    OR COALESCE(properties ->> 'excerpt', '') ILIKE '%' || $1 || '%'
			    OR COALESCE(properties::TEXT, '') ILIKE '%' || $1 || '%'
			  )
			ORDER BY properties ->> 'publishing_date' DESC
			LIMIT 20
		`, [q.trim()]);
	}

	return {
		query: q,
		tag: tagPath,
		articles,
		categories: parentData.categories,
		tags: parentData.tags
	};
}
