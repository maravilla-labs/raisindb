import { useState, useEffect } from 'react';
import { useRaisinClient } from '../hooks/useRaisinClient';
import { getConfig } from '../lib/raisin';

export default function Admin() {
  const { client } = useRaisinClient();
  const [repositories, setRepositories] = useState<any[]>([]);
  const [nodeTypes, setNodeTypes] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    async function fetchData() {
      if (!client) return;

      setLoading(true);
      try {
        const config = getConfig();

        // Fetch repositories
        const repos = await client.listRepositories();
        setRepositories(repos);

        // Fetch NodeTypes
        const db = client.database(config.repository);
        const types = await db.nodeTypes().list();
        setNodeTypes(types);
      } catch (error) {
        console.error('Failed to fetch admin data:', error);
      } finally {
        setLoading(false);
      }
    }

    fetchData();
  }, [client]);

  if (loading) {
    return (
      <div className="text-center py-8">
        <div className="inline-block animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
      </div>
    );
  }

  return (
    <div className="max-w-4xl mx-auto">
      <h1 className="text-3xl font-bold mb-6">Admin Panel</h1>

      {/* Repositories */}
      <div className="card mb-6">
        <h2 className="text-xl font-bold mb-4">Repositories</h2>
        {repositories.length === 0 ? (
          <p className="text-gray-500">No repositories found</p>
        ) : (
          <div className="space-y-2">
            {repositories.map((repo: any, index) => (
              <div
                key={index}
                className="p-3 border border-gray-200 rounded"
              >
                <div className="font-semibold">{repo.repository_id || repo.name}</div>
                <div className="text-sm text-gray-600">
                  {repo.description || 'No description'}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* NodeTypes */}
      <div className="card mb-6">
        <h2 className="text-xl font-bold mb-4">NodeTypes</h2>
        {nodeTypes.length === 0 ? (
          <p className="text-gray-500">No NodeTypes found</p>
        ) : (
          <div className="space-y-2">
            {nodeTypes.map((nodeType: any, index) => (
              <div
                key={index}
                className="p-3 border border-gray-200 rounded"
              >
                <div className="font-semibold">{nodeType.name}</div>
                <div className="text-sm text-gray-600">
                  {nodeType.description || 'No description'}
                </div>
                <div className="mt-2 text-xs text-gray-500">
                  Properties: {Object.keys(nodeType.properties || {}).join(', ')}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* API Examples */}
      <div className="card">
        <h2 className="text-xl font-bold mb-4">API Examples</h2>
        <div className="space-y-4">
          <div>
            <h3 className="font-semibold mb-2">SQL Query</h3>
            <pre className="p-3 bg-gray-50 rounded text-sm overflow-x-auto">
              {`const result = await db.executeSql(
  "SELECT * FROM nodes WHERE node_type = 'Post'"
);`}
            </pre>
          </div>
          <div>
            <h3 className="font-semibold mb-2">Graph Relationship (Cypher-style)</h3>
            <pre className="p-3 bg-gray-50 rounded text-sm overflow-x-auto">
              {`// Add follow relationship
await ws.nodes().addRelation(
  followerId,
  followingId,
  workspace,
  'follows',
  'SocialUser'
);`}
            </pre>
          </div>
          <div>
            <h3 className="font-semibold mb-2">Real-time Subscription</h3>
            <pre className="p-3 bg-gray-50 rounded text-sm overflow-x-auto">
              {`const { data, loading } = useLiveQuery({
  nodeType: 'Post',
  path: '/posts/%'
});`}
            </pre>
          </div>
        </div>
      </div>
    </div>
  );
}
