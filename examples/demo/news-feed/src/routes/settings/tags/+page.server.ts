import { fail } from '@sveltejs/kit';
import { executeWithUser, query } from '$lib/server/db';
import { TAGS_PATH } from '$lib/types';
import type { TagNode, TagProperties } from '$lib/types';

// Helper to build hierarchical tag tree from flat results
function buildTagTree(rows: Array<{ id: string; path: string; name: string; node_type: string; properties: TagProperties }>): TagNode[] {
	const nodeMap = new Map<string, TagNode>();
	const roots: TagNode[] = [];

	// First pass: create all nodes
	for (const row of rows) {
		nodeMap.set(row.path, {
			id: row.id,
			path: row.path,
			name: row.name,
			node_type: row.node_type,
			properties: row.properties,
			children: []
		});
	}

	// Second pass: build tree structure
	for (const node of nodeMap.values()) {
		const parentPath = node.path.substring(0, node.path.lastIndexOf('/'));
		const parent = nodeMap.get(parentPath);
		if (parent) {
			parent.children = parent.children || [];
			parent.children.push(node);
		} else if (node.path.startsWith(TAGS_PATH + '/') && node.path.split('/').length === TAGS_PATH.split('/').length + 1) {
			// Top-level tag (direct child of /superbigshit/tags)
			roots.push(node);
		}
	}

	return roots;
}

export async function load() {
	// Fetch all tags under /superbigshit/tags (excluding the root itself)
	const rows = await query<{ id: string; path: string; name: string; node_type: string; properties: TagProperties }>(`
		SELECT id, path, name, node_type, properties
		FROM social
		WHERE DESCENDANT_OF($1)
		  AND node_type = 'news:Tag'
		ORDER BY path
	`, [TAGS_PATH]);

	const tags = buildTagTree(rows);

	return { tags };
}

export const actions = {
	create: async ({ request, locals }) => {
		const accessToken = locals.accessToken;
		if (!accessToken) {
			return fail(401, { error: 'Authentication required' });
		}

		const formData = await request.formData();
		const name = formData.get('name') as string;
		const parentPath = formData.get('parentPath') as string;
		const label = formData.get('label') as string;
		const icon = formData.get('icon') as string;
		const color = formData.get('color') as string;

		if (!name || !label) {
			return fail(400, { error: 'Name and label are required' });
		}

		const path = `${parentPath}/${name}`;
		const properties: TagProperties = { label };
		if (icon) properties.icon = icon;
		if (color) properties.color = color;

		try {
			await executeWithUser(`
				INSERT INTO social (path, node_type, name, properties)
				VALUES ($1, 'news:Tag', $2, $3::JSONB)
			`, [path, name, JSON.stringify(properties)], accessToken);

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
		const label = formData.get('label') as string;
		const icon = formData.get('icon') as string;
		const color = formData.get('color') as string;

		if (!path || !label) {
			return fail(400, { error: 'Path and label are required' });
		}

		const properties: TagProperties = { label };
		if (icon) properties.icon = icon;
		if (color) properties.color = color;

		try {
			await executeWithUser(`
				UPDATE social
				SET properties = $1::JSONB
				WHERE path = $2
			`, [JSON.stringify(properties), path], accessToken);

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
			// Delete the tag (this will fail if it has children due to RaisinDB constraints)
			await executeWithUser(`
				DELETE FROM social WHERE path = $1
			`, [path], accessToken);

			return { success: true };
		} catch (err) {
			return fail(500, { error: (err as Error).message });
		}
	}
};
