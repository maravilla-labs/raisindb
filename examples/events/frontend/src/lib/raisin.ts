import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';
import type { SqlResult } from '@raisindb/client';

const WS_URL = 'ws://localhost:8081/sys/default/events';
const REPO = 'events';
const WORKSPACE = 'events';

let clientInstance: RaisinClient | null = null;

export function getClient(): RaisinClient {
	if (!clientInstance) {
		clientInstance = new RaisinClient(WS_URL, {
			tokenStorage: new LocalStorageTokenStorage(REPO),
			tenantId: 'default',
			defaultBranch: 'main',
			connection: {
				autoReconnect: true,
				heartbeatInterval: 30000,
			},
			requestTimeout: 30000,
		});
	}
	return clientInstance;
}

let connectionResolve: (() => void) | null = null;
const connectionPromise = new Promise<void>((resolve) => {
	connectionResolve = resolve;
});

export async function initSession() {
	const client = getClient();
	try {
		const user = await client.initSession(REPO);
		connectionResolve?.();
		return user;
	} catch {
		// If initSession fails, connect anonymously
		if (!client.isConnected()) {
			await client.connect();
		}
		connectionResolve?.();
		return null;
	}
}

export async function getDatabase() {
	await connectionPromise;
	return getClient().database(REPO);
}

export async function query<T>(sql: string, params?: unknown[]): Promise<T[]> {
	const db = await getDatabase();
	const result: SqlResult = await db.executeSql(sql, params);
	return (result.rows ?? []) as T[];
}

export async function queryOne<T>(sql: string, params?: unknown[]): Promise<T | null> {
	const rows = await query<T>(sql, params);
	return rows[0] ?? null;
}

export async function getWorkspace() {
	const db = await getDatabase();
	return db.workspace(WORKSPACE);
}

export { REPO, WORKSPACE };
