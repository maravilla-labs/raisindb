export interface Event {
	id: string;
	path: string;
	node_type: string;
	archetype?: string;
	properties: {
		title: string;
		slug: string;
		description?: string;
		start_date: string;
		end_date?: string;
		location?: string;
		category?: string;
		tags?: string[];
		capacity?: number;
		price?: number;
		currency?: string;
		status?: string;
		featured?: boolean;
		cover_image?: ResourceValue;
	};
}

export interface Venue {
	id: string;
	path: string;
	node_type: string;
	properties: {
		title: string;
		slug: string;
		description?: string;
		address?: string;
		city?: string;
		country?: string;
		capacity?: number;
		image?: ResourceValue;
		website?: string;
		contact_email?: string;
	};
}

export interface Speaker {
	id: string;
	path: string;
	node_type: string;
	properties: {
		name: string;
		slug: string;
		bio?: string;
		title?: string;
		company?: string;
		photo?: ResourceValue;
		website?: string;
		twitter?: string;
		linkedin?: string;
	};
}

export interface ContentElement {
	element_type: string;
	[key: string]: unknown;
}

export interface Page {
	id: string;
	path: string;
	node_type: string;
	archetype?: string;
	properties: {
		title: string;
		slug: string;
		description?: string;
		content?: ContentElement[];
	};
}

export interface ResourceValue {
	storage_key?: string;
	filename?: string;
	mime_type?: string;
	size?: number;
	url?: string;
}
