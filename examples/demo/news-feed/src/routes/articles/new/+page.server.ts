import { redirect, fail } from '@sveltejs/kit';
import { executeWithUser } from '$lib/server/db';
import { ARTICLES_PATH, type RaisinReference } from '$lib/types';

export async function load({ parent, locals }) {
	// Require authentication to create articles
	if (!locals.user) {
		redirect(303, '/auth/login?redirect=/articles/new');
	}

	const parentData = await parent();
	return {
		categories: parentData.categories,
		tags: parentData.tags,
		user: locals.user
	};
}

export const actions = {
	default: async ({ request, locals }) => {
		// Require authentication
		if (!locals.user) {
			redirect(303, '/auth/login');
		}

		const formData = await request.formData();

		const title = formData.get('title') as string;
		const slug = formData.get('slug') as string;
		const category = formData.get('category') as string;
		const excerpt = formData.get('excerpt') as string;
		const body = formData.get('body') as string;
		const tagsRaw = formData.get('tags') as string;
		const keywordsRaw = formData.get('keywords') as string;
		const featured = formData.get('featured') === 'on';
		const status = formData.get('status') as string;
		const publishingDateRaw = formData.get('publishing_date') as string;
		const imageUrl = formData.get('imageUrl') as string;

		// Use authenticated user's display name or email as author
		const author = locals.user.displayName || locals.user.email;

		// Convert datetime-local to ISO string for storage
		const publishingDate = publishingDateRaw
			? new Date(publishingDateRaw).toISOString()
			: new Date().toISOString();

		if (!title || !slug || !category) {
			return fail(400, { error: 'Title, slug, and category are required' });
		}

		// Parse tags as RaisinReference objects (JSON array from form)
		// Format: [{"raisin:ref":"id","raisin:workspace":"social","raisin:path":"/superbigshit/tags/..."}]
		let tags: RaisinReference[] = [];
		if (tagsRaw) {
			try {
				tags = JSON.parse(tagsRaw);
			} catch {
				// Fallback: treat as comma-separated paths and convert to references
				// This handles legacy format or manual input
				tags = [];
			}
		}

		// Parse keywords as simple string array
		const keywords = keywordsRaw
			? keywordsRaw.split(',').map((k) => k.trim()).filter(Boolean)
			: [];

		const path = `${ARTICLES_PATH}/${category}/${slug}`;
		const properties = {
			title,
			slug,
			excerpt: excerpt || '',
			body: body || '',
			tags,        // RaisinReference[] - proper references
			keywords,    // string[] - simple keywords for search
			featured,
			status: status || 'published',
			publishing_date: publishingDate,
			views: 0,
			author: author || '',
			imageUrl: imageUrl || ''
		};

		try {
			// Require access token for write operations
			if (!locals.accessToken) {
				return fail(401, { error: 'Authentication required to create articles' });
			}

			const sql = `
				INSERT INTO social (path, node_type, name, properties)
				VALUES ($1, 'news:Article', $2, $3::JSONB)
			`;
			const params = [path, title, JSON.stringify(properties)];

			// Execute with user context for row-level security
			await executeWithUser(sql, params, locals.accessToken);

			throw redirect(303, `/articles/${category}/${slug}`);
		} catch (err) {
			if ((err as { status?: number }).status === 303) {
				throw err;
			}
			return fail(500, { error: (err as Error).message });
		}
	}
};
