import { json } from '@sveltejs/kit';
import { pool } from '$lib/server/db';

export async function GET() {
	return json({
		totalCount: pool.totalCount,    // Total connections (idle + in use)
		idleCount: pool.idleCount,      // Connections currently idle
		waitingCount: pool.waitingCount // Requests waiting for a connection
	});
}
