import pg from 'pg';

const { Pool } = pg;

const pool = new Pool({
	connectionString:
		'postgresql://default:raisin_N7U7POgxOh9WqZIaPC5YK1W23HlEieb9@localhost:5432/social_feed_demo_rel4',
	min: 5,
	max: 100,
	// Connection timeout - fail fast if DB is down
	connectionTimeoutMillis: 5000,
	// Idle timeout - remove idle connections after 30 seconds
	idleTimeoutMillis: 30000,
});

// Track pool health status
let poolHealthy = true;
let lastHealthCheck = Date.now();

// Handle pool errors gracefully - prevents process crash
pool.on('error', (err: Error) => {
	console.error('Unexpected pool error on idle client:', err.message);
	poolHealthy = false;
	// Don't crash the process - the pool will handle reconnection
});

// Log when connections are removed
pool.on('remove', () => {
	console.log(`Pool connection removed. Total: ${pool.totalCount}, Idle: ${pool.idleCount}`);
});

// Pre-warm the pool by creating min connections eagerly
async function warmPool() {
	try {
		const clients = await Promise.all(
			Array.from({ length: 10 }, () =>
				pool.connect().catch(() => null)
			)
		);
		const successfulClients = clients.filter(c => c !== null);
		successfulClients.forEach(client => client?.release());

		if (successfulClients.length > 0) {
			console.log(`Pool warmed up with ${pool.totalCount} connections`);
			poolHealthy = true;
		} else {
			console.warn('Pool warmup failed - database may be unavailable');
			poolHealthy = false;
		}
	} catch (err) {
		console.warn('Pool warmup error:', err instanceof Error ? err.message : err);
		poolHealthy = false;
	}
}

warmPool();

/**
 * Check if the pool is healthy, with periodic retry
 */
async function ensurePoolHealthy(): Promise<void> {
	const now = Date.now();
	// Only check health every 5 seconds at most
	if (!poolHealthy && now - lastHealthCheck > 5000) {
		lastHealthCheck = now;
		try {
			const client = await pool.connect();
			client.release();
			poolHealthy = true;
			console.log('Database connection restored');
		} catch {
			poolHealthy = false;
		}
	}
}

/**
 * Get a client with retry logic
 */
async function getClientWithRetry(retries = 3, delay = 1000): Promise<pg.PoolClient> {
	let lastError: Error | undefined;

	for (let i = 0; i < retries; i++) {
		try {
			await ensurePoolHealthy();
			return await pool.connect();
		} catch (err) {
			lastError = err instanceof Error ? err : new Error(String(err));
			poolHealthy = false;

			if (i < retries - 1) {
				console.warn(`Database connection attempt ${i + 1} failed, retrying in ${delay}ms...`);
				await new Promise(resolve => setTimeout(resolve, delay));
			}
		}
	}

	throw new Error(`Database connection failed after ${retries} attempts: ${lastError?.message}`);
}

/**
 * Check if an error is a connection-related error
 */
function isConnectionError(err: unknown): boolean {
	if (err instanceof Error) {
		const msg = err.message.toLowerCase();
		return (
			msg.includes('connection terminated') ||
			msg.includes('connection refused') ||
			msg.includes('econnreset') ||
			msg.includes('econnrefused') ||
			msg.includes('timeout') ||
			msg.includes('client has encountered a connection error')
		);
	}
	return false;
}

/**
 * Execute a query without identity context (admin-level access)
 */
export async function query<T = Record<string, unknown>>(
	text: string,
	params?: unknown[]
): Promise<T[]> {
	const client = await getClientWithRetry();

	try {
		console.log('Executing query:', text, params);
		const result = await client.query(text, params);
		return result.rows as T[];
	} catch (error) {
		if (isConnectionError(error)) {
			poolHealthy = false;
		}
		console.error('Database query error:', error);
		throw error;
	} finally {
		client.release();
	}
}

/**
 * Execute a query with identity user context.
 * Sets the user's JWT token via SET app.user before executing the query,
 * enabling row-level security based on the user's identity.
 *
 * @param text - SQL query
 * @param params - Query parameters
 * @param accessToken - The identity user's JWT access token
 */
export async function queryWithUser<T = Record<string, unknown>>(
	text: string,
	params: unknown[] | undefined,
	accessToken: string
): Promise<T[]> {
	const client = await getClientWithRetry();

	try {
		// Set the identity context for this connection
		await client.query(`SET app.user = $1`, [accessToken]);

		console.log('Executing query with user context:', text, params);
		const result = await client.query(text, params);
		return result.rows as T[];
	} catch (error) {
		if (isConnectionError(error)) {
			poolHealthy = false;
		}
		console.error('Database query error:', error);
		throw error;
	} finally {
		// Reset the identity context before releasing
		try {
			await client.query('RESET app.user');
		} catch {
			// Ignore reset errors
		}
		client.release();
	}
}

/**
 * Execute a query with optional identity context.
 * If accessToken is provided, uses queryWithUser, otherwise uses query.
 */
export async function queryMaybeUser<T = Record<string, unknown>>(
	text: string,
	params?: unknown[],
	accessToken?: string | null
): Promise<T[]> {
	if (accessToken) {
		return queryWithUser<T>(text, params, accessToken);
	}
	return query<T>(text, params);
}

export async function queryOne<T = Record<string, unknown>>(
	text: string,
	params?: unknown[]
): Promise<T | null> {
	const rows = await query<T>(text, params);
	return rows[0] ?? null;
}

/**
 * Execute a query with identity context and return first result
 */
export async function queryOneWithUser<T = Record<string, unknown>>(
	text: string,
	params: unknown[] | undefined,
	accessToken: string
): Promise<T | null> {
	const rows = await queryWithUser<T>(text, params, accessToken);
	return rows[0] ?? null;
}

/**
 * Execute a query with optional identity context and return first result.
 * If accessToken is provided, uses queryWithUser, otherwise uses query.
 */
export async function queryOneMaybeUser<T = Record<string, unknown>>(
	text: string,
	params?: unknown[],
	accessToken?: string | null
): Promise<T | null> {
	const rows = await queryMaybeUser<T>(text, params, accessToken);
	return rows[0] ?? null;
}

/**
 * Execute a statement with optional identity context.
 * If accessToken is provided, uses executeWithUser, otherwise uses execute.
 */
export async function executeMaybeUser(
	text: string,
	params?: unknown[],
	accessToken?: string | null
): Promise<number> {
	if (accessToken) {
		return executeWithUser(text, params, accessToken);
	}
	return execute(text, params);
}

export async function execute(text: string, params?: unknown[]): Promise<number> {
	const client = await getClientWithRetry();
	try {
		const result = await client.query(text, params);
		await new Promise(resolve => setTimeout(resolve, 200));
		return result.rowCount ?? 0;
	} catch (error) {
		if (isConnectionError(error)) {
			poolHealthy = false;
		}
		throw error;
	} finally {
		client.release();
	}
}

/**
 * Execute a statement with identity user context.
 */
export async function executeWithUser(
	text: string,
	params: unknown[] | undefined,
	accessToken: string
): Promise<number> {
	const client = await getClientWithRetry();
	try {
		// Set the identity context for this connection
		await client.query(`SET app.user = $1`, [accessToken]);

		const result = await client.query(text, params);
		await new Promise(resolve => setTimeout(resolve, 200));
		return result.rowCount ?? 0;
	} catch (error) {
		if (isConnectionError(error)) {
			poolHealthy = false;
		}
		throw error;
	} finally {
		// Reset the identity context before releasing
		try {
			await client.query('RESET app.user');
		} catch {
			// Ignore reset errors
		}
		client.release();
	}
}

/**
 * Check if the database pool is healthy
 */
export function isPoolHealthy(): boolean {
	return poolHealthy;
}

/**
 * Get pool statistics
 */
export function getPoolStats() {
	return {
		healthy: poolHealthy,
		totalCount: pool.totalCount,
		idleCount: pool.idleCount,
		waitingCount: pool.waitingCount,
	};
}

export { pool };
