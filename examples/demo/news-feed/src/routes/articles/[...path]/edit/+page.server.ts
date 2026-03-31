import { error, redirect, fail } from '@sveltejs/kit';
import { queryOne, executeWithUser, query, queryWithUser } from '$lib/server/db';
import {
	ARTICLES_PATH,
	type Article,
	type ArticleConnection,
	type IncomingConnection,
	getCategoryFromPath,
	type RaisinReference
} from '$lib/types';

export async function load({ params, parent }) {
	const fullDbPath = `${ARTICLES_PATH}/${params.path}`;
	const workspacePath = `social:${fullDbPath}`;
	const parentData = await parent();

	// Load article
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

	// Load all articles for the connection picker (excluding current)
	const allArticles = await query<Article>(`
		SELECT id, path, name, node_type, properties, created_at, updated_at
		FROM social
		WHERE node_type = 'news:Article'
		  AND path != $1
		ORDER BY properties ->> 'title'
	`, [fullDbPath]);

	// Load incoming connections using NEIGHBORS (simpler for single-hop)
	const incomingRaw = await query<{
		id: string;
		path: string;
		properties: Article['properties'];
		relation_type: string;
		weight: number;
	}>(`
		SELECT n.id, n.path, n.properties, n.relation_type, n.weight
		FROM NEIGHBORS('social:${fullDbPath.replace(/'/g, "''")}', 'IN', NULL) AS n
		WHERE n.node_type = 'news:Article'
	`);

	const incomingConnections: IncomingConnection[] = incomingRaw.map(row => ({
		sourceId: row.id,
		sourcePath: row.path,
		sourceTitle: row.properties?.title || 'Unknown',
		relationType: row.relation_type as IncomingConnection['relationType'],
		weight: Math.round((row.weight || 0.75) * 100)
	}));

	return {
		article,
		currentCategory,
		categories: parentData.categories,
		tags: parentData.tags,
		availableArticles: allArticles,
		incomingConnections
	};
}

export const actions = {
	default: async ({ request, params, locals }) => {
		const formData = await request.formData();
		const originalPath = `${ARTICLES_PATH}/${params.path}`;
		const accessToken = locals.accessToken;

		// Require authentication for write operations
		if (!accessToken) {
			return fail(401, { error: 'Authentication required to edit articles' });
		}

		const title = formData.get('title') as string;
		const slug = formData.get('slug') as string;
		const category = formData.get('category') as string;
		const excerpt = formData.get('excerpt') as string;
		const body = formData.get('body') as string;
		const tagsRaw = formData.get('tags') as string;
		const connectionsRaw = formData.get('connections') as string;
		const keywordsRaw = formData.get('keywords') as string;
		const featured = formData.get('featured') === 'on';
		const status = formData.get('status') as string;
		const publishingDateRaw = formData.get('publishing_date') as string;
		const author = formData.get('author') as string;
		const imageUrl = formData.get('imageUrl') as string;

		// Convert datetime-local to ISO string for storage
		const publishingDate = publishingDateRaw
			? new Date(publishingDateRaw).toISOString()
			: new Date().toISOString();

		if (!title || !slug || !category) {
			return fail(400, { error: 'Title, slug, and category are required' });
		}

		// Parse tags as RaisinReference objects (JSON array from form)
		let tags: RaisinReference[] = [];
		if (tagsRaw) {
			try {
				tags = JSON.parse(tagsRaw);
			} catch {
				tags = [];
			}
		}

		// Parse connections from form
		let newConnections: ArticleConnection[] = [];
		if (connectionsRaw) {
			try {
				newConnections = JSON.parse(connectionsRaw);
			} catch {
				newConnections = [];
			}
		}

		// Parse keywords as simple string array
		const keywords = keywordsRaw
			? keywordsRaw.split(',').map((k) => k.trim()).filter(Boolean)
			: [];

		const newPath = `${ARTICLES_PATH}/${category}/${slug}`;
		const properties = {
			title,
			slug,
			excerpt: excerpt || '',
			body: body || '',
			tags,
			keywords,
			featured,
			status: status || 'published',
			publishing_date: publishingDate,
			author: author || '',
			imageUrl: imageUrl || '',
			connections: newConnections // Store in properties for quick access
		};

		try {
			// Get existing outgoing connections from graph via NEIGHBORS
			const existingRelations = await query<{
				path: string;
				relation_type: string;
			}>(`
				SELECT n.path, n.relation_type
				FROM NEIGHBORS('social:${originalPath.replace(/'/g, "''")}', 'OUT', NULL) AS n
			`);

			// Build sets for comparison
			const existingSet = new Set(
				existingRelations.map(r => `${r.path}|${r.relation_type}`)
			);
			const newSet = new Set(
				newConnections.map(c => `${c.targetPath}|${c.relationType}`)
			);

			// Find relations to remove (in existing but not in new)
			const toRemove = existingRelations.filter(
				r => !newSet.has(`${r.path}|${r.relation_type}`)
			);

			// Find relations to add (in new but not in existing)
			const toAdd = newConnections.filter(
				c => !existingSet.has(`${c.targetPath}|${c.relationType}`)
			);

			// Find relations to update (weight changed)
			const toUpdate = newConnections.filter(c => {
				const key = `${c.targetPath}|${c.relationType}`;
				return existingSet.has(key);
			});

			// Execute UNRELATE for removed connections
			for (const rel of toRemove) {
				await executeWithUser(`
					UNRELATE FROM path='${originalPath}' IN WORKSPACE 'social'
					  TO path='${rel.path}' IN WORKSPACE 'social'
					  TYPE '${rel.relation_type}'
				`, undefined, accessToken);
			}

			// Execute RELATE for new connections
			for (const conn of toAdd) {
				const weight = conn.weight / 100; // Convert 0-100 to 0-1
				await executeWithUser(`
					RELATE FROM path='${originalPath}' IN WORKSPACE 'social'
					  TO path='${conn.targetPath}' IN WORKSPACE 'social'
					  TYPE '${conn.relationType}' WEIGHT ${weight}
				`, undefined, accessToken);
			}

			// Update existing relations (remove and re-add with new weight)
			for (const conn of toUpdate) {
				const weight = conn.weight / 100;
				await executeWithUser(`
					UNRELATE FROM path='${originalPath}' IN WORKSPACE 'social'
					  TO path='${conn.targetPath}' IN WORKSPACE 'social'
					  TYPE '${conn.relationType}'
				`, undefined, accessToken);
				await executeWithUser(`
					RELATE FROM path='${originalPath}' IN WORKSPACE 'social'
					  TO path='${conn.targetPath}' IN WORKSPACE 'social'
					  TYPE '${conn.relationType}' WEIGHT ${weight}
				`, undefined, accessToken);
			}

			// Check if path changed (category or slug changed)
			if (originalPath !== newPath) {
				// Move the article to new path
				await executeWithUser(`
					MOVE social SET path = $1 TO path = $2
				`, [originalPath, `${ARTICLES_PATH}/${category}/`], accessToken);

				// Then update the name/slug if needed
				await executeWithUser(`
					UPDATE social
					SET name = $1,
					    properties = properties || $2::JSONB
					WHERE path = $3
				`, [title, JSON.stringify(properties), newPath], accessToken);
			} else {
				// Just update properties
				await executeWithUser(`
					UPDATE social
					SET name = $1,
					    properties = properties || $2::JSONB
					WHERE path = $3
				`, [title, JSON.stringify(properties), originalPath], accessToken);
			}

			throw redirect(303, `/articles/${category}/${slug}`);
		} catch (err) {
			if ((err as { status?: number }).status === 303) {
				throw err;
			}
			return fail(500, { error: (err as Error).message });
		}
	}
};
