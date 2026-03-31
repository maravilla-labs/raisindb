import { RaisinClient } from '@raisindb/client';

const HTTP_URL = 'http://localhost:8081';
const REPO = 'events';

const httpClient = RaisinClient.forSSR(HTTP_URL, {
	tenantId: 'default',
	defaultBranch: 'main',
});

const db = httpClient.database(REPO);

export async function serverQuery<T>(sql: string, params?: unknown[]): Promise<T[]> {
	const result = await db.executeSql(sql, params);
	return (result.rows ?? []) as T[];
}

export async function serverQueryOne<T>(sql: string, params?: unknown[]): Promise<T | null> {
	const rows = await serverQuery<T>(sql, params);
	return rows[0] ?? null;
}
