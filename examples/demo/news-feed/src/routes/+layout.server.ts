import { queryMaybeUser } from '$lib/server/db';
import { ARTICLES_PATH, TAGS_PATH, type Category, type TagNode, type TagProperties } from '$lib/types';

interface CategoryRow {
	id: string;
	path: string;
	name: string;
	properties: {
		label: string;
		color: string;
		order: number;
	};
}

interface TagRow {
	id: string;
	path: string;
	name: string;
	node_type: string;
	properties: TagProperties;
}

// Helper to build hierarchical tag tree from flat results
function buildTagTree(rows: TagRow[]): TagNode[] {
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

export async function load({ locals }) {
	// Use identity context if user is logged in (enables RLS)
	const accessToken = locals.accessToken;

	// Fetch categories dynamically from database
	const categoryRows = await queryMaybeUser<CategoryRow>(`
		SELECT id, path, name, properties
		FROM social
		WHERE CHILD_OF('${ARTICLES_PATH}')
		  AND node_type = 'raisin:Folder'
	`, [], accessToken);

	const categories: Category[] = categoryRows.map((row) => ({
		id: row.id,
		path: row.path,
		name: row.name,
		slug: row.path.split('/').pop() || '',
		properties: row.properties
	}));

	// Fetch all tags
	const tagRows = await queryMaybeUser<TagRow>(`
		SELECT id, path, name, node_type, properties
		FROM social
		WHERE DESCENDANT_OF($1)
		  AND node_type = 'news:Tag'
		ORDER BY path
	`, [TAGS_PATH], accessToken);

	const tags = buildTagTree(tagRows);

	return {
		categories,
		tags,
		user: locals.user
	};
}
