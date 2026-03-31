import { writable, derived } from 'svelte/store';
import type { User } from './users';

// Current authenticated user
export const currentUser = writable<User | null>(null);

// Derived: is logged in
export const isAuthenticated = derived(currentUser, ($user) => $user !== null);

// Derived: role checks
export const isCustomer = derived(currentUser, ($user) => $user?.role === 'customer');
export const isNurse = derived(currentUser, ($user) => $user?.role === 'nurse');

// Auth actions
export function login(user: User): void {
	currentUser.set(user);
	if (typeof localStorage !== 'undefined') {
		localStorage.setItem('pocketmedico_user', JSON.stringify(user));
	}
}

export function logout(): void {
	currentUser.set(null);
	if (typeof localStorage !== 'undefined') {
		localStorage.removeItem('pocketmedico_user');
	}
}

// Restore session from localStorage
export function restoreSession(): void {
	if (typeof localStorage !== 'undefined') {
		const stored = localStorage.getItem('pocketmedico_user');
		if (stored) {
			try {
				const user = JSON.parse(stored) as User;
				currentUser.set(user);
			} catch {
				localStorage.removeItem('pocketmedico_user');
			}
		}
	}
}
