import { writable } from 'svelte/store';

export type UserRole = 'customer' | 'nurse';

export interface User {
	id: string;
	email: string;
	displayName: string;
	role: UserRole;
	practiceName?: string; // For customers
	specialization?: string; // For customers
}

export interface MockUser extends User {
	password: string;
}

// Initial demo users
const initialUsers: MockUser[] = [
	{
		id: 'customer-demo',
		email: 'doctor@demo.com',
		password: 'demo1234',
		displayName: 'Dr. Schmidt',
		role: 'customer',
		practiceName: 'Praxis Schmidt',
		specialization: 'General Medicine'
	},
	{
		id: 'nurse-demo',
		email: 'nurse@demo.com',
		password: 'demo1234',
		displayName: 'Anna Meier',
		role: 'nurse'
	}
];

// Mock users database
export const users = writable<MockUser[]>(initialUsers);

// Find user by email
export function findUserByEmail(email: string): MockUser | undefined {
	let found: MockUser | undefined;
	users.subscribe((u) => {
		found = u.find((user) => user.email.toLowerCase() === email.toLowerCase());
	})();
	return found;
}

// Register new user
export function registerUser(user: Omit<MockUser, 'id'>): MockUser {
	const newUser: MockUser = {
		...user,
		id: crypto.randomUUID()
	};
	users.update((u) => [...u, newUser]);
	return newUser;
}

// Validate login credentials
export function validateCredentials(email: string, password: string): MockUser | null {
	const user = findUserByEmail(email);
	if (user && user.password === password) {
		return user;
	}
	return null;
}
