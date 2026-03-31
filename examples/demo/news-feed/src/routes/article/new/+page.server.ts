import { redirect, fail } from '@sveltejs/kit';
import { query } from '$lib/server/db';
import { slugify } from '$lib/utils';

export const actions = {
	default: async ({ request }) => {
		const formData = await request.formData();

		const title = formData.get('title') as string;
		const slug = formData.get('slug') as string || slugify(title);
		const excerpt = formData.get('excerpt') as string || '';
		const body = formData.get('body') as string || '';
		const category = formData.get('category') as string;
		const tagsInput = formData.get('tags') as string || '';
		const featured = formData.get('featured') === 'on';
		const status = formData.get('status') as string || 'published';
		const author = formData.get('author') as string || '';
		const imageUrl = formData.get('imageUrl') as string || '';

		if (!title || !category) {
			return fail(400, { error: 'Title and category are required' });
		}

		const tags = tagsInput
			.split(',')
			.map((t) => t.trim())
			.filter(Boolean);

		const path = `/superbigshit/articles/${category}/${slug}`;
		const properties = JSON.stringify({
			title,
			slug,
			excerpt,
			body,
			category,
			tags,
			featured,
			status,
			views: 0,
			author,
			imageUrl: imageUrl || null
		});

		try {
			const result = await query<{ id: string }>(`
				INSERT INTO social (path, node_type, name, properties)
				VALUES ($1, 'news:Article', $2, $3::jsonb)
				RETURNING id
			`, [path, title, properties]);

			if (result.length > 0) {
				throw redirect(303, `/article/${result[0].id}`);
			}

			return fail(500, { error: 'Failed to create article' });
		} catch (err) {
			if ((err as { status?: number }).status === 303) throw err;
			return fail(400, { error: (err as Error).message });
		}
	}
};
