import { serverQueryOne } from '$lib/raisin.server';
import type { Page } from '$lib/types';

export async function load() {
	const page = await serverQueryOne<Page>(
		"SELECT id, path, node_type, archetype, properties FROM 'events' WHERE node_type = $1 AND properties->>'slug'::String = $2",
		['events:Page', 'home']
	);

	return { page };
}
