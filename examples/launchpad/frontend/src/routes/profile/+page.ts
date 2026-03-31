import type { PageLoad } from './$types';
import { queryOne } from '$lib/raisin';

interface ProfileNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    data: Record<string, string>;
  };
}

const USER_WORKSPACE = 'raisin:access_control';

export const load: PageLoad = async ({ parent }) => {
  // Get user from parent layout
  const { user } = await parent();

  if (!user?.home) {
    return {
      privateProfile: null,
      publicProfile: null,
      error: 'Not logged in or no home path',
    };
  }

  const privatePath = `${user.home}/profile/private`;
  const publicPath = `${user.home}/profile/public`;

  try {
    const [privateProfile, publicProfile] = await Promise.all([
      queryOne<ProfileNode>(`
        SELECT id, path, name, node_type, properties
        FROM "${USER_WORKSPACE}"
        WHERE path = $1
      `, [privatePath]),
      queryOne<ProfileNode>(`
        SELECT id, path, name, node_type, properties
        FROM "${USER_WORKSPACE}"
        WHERE path = $1
      `, [publicPath]),
    ]);

    return {
      privateProfile,
      publicProfile,
      error: null,
    };
  } catch (error) {
    console.error('[profile] Load error:', error);
    return {
      privateProfile: null,
      publicProfile: null,
      error: error instanceof Error ? error.message : 'Failed to load profile',
    };
  }
};
