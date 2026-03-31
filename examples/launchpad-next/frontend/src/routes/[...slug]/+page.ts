import type { PageLoad } from './$types';
import { getPageByPath } from '$lib/raisin';

export const load: PageLoad = async ({ params }) => {
  const slug = params.slug || 'home';
  const path = `/${slug}`;

  try {
    const page = await getPageByPath(path);
    return { page };
  } catch (error) {
    console.error(`Failed to load page: ${path}`, error);
    return {
      page: null,
      error: error instanceof Error ? error.message : 'Page not found'
    };
  }
};
