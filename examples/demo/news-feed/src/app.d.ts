// See https://svelte.dev/docs/kit/types#app.d.ts
// for information about these interfaces

interface User {
	id: string;
	email: string;
	displayName: string | null;
	avatarUrl: string | null;
	emailVerified: boolean;
}

declare global {
	namespace App {
		// interface Error {}
		interface Locals {
			user: User | null;
			/** JWT access token for identity user queries */
			accessToken: string | null;
		}
		interface PageData {
			user: User | null;
		}
		// interface PageState {}
		// interface Platform {}
	}
}

export {};
