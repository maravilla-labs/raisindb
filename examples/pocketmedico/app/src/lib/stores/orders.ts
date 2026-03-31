import { writable, derived, get } from 'svelte/store';
import { currentUser } from './auth';

export type OrderStatus =
	| 'pending' // Waiting for files
	| 'processing' // AI transcribing
	| 'ai_complete' // AI done, waiting for review (Pro) or ready (Light)
	| 'in_review' // Nurse reviewing (Pro tier)
	| 'ready' // Ready for download
	| 'downloaded' // Downloaded by customer
	| 'error'; // Processing error

export type ServiceTier = 'light' | 'pro';

// Output document types (what we generate)
export type OutputDocumentType =
	| 'arztbrief' // Medical Letter
	| 'befundbericht' // Findings Report
	| 'entlassungsbericht' // Discharge Summary
	| 'konsiliarbericht' // Consultation Report
	| 'ueberweisungsbrief'; // Referral Letter

// Input document types (what user uploads - legacy compatibility)
export type DocumentType =
	| 'medical_report'
	| 'discharge_summary'
	| 'consultation_note'
	| 'prescription'
	| 'other';

// Block types for structured transcripts
export type BlockType =
	// SOAP Format (International)
	| 'subjective'
	| 'objective'
	| 'assessment'
	| 'plan'
	// Swiss Format (German Medical)
	| 'kopfdaten'
	| 'anamnese'
	| 'befund'
	| 'diagnose'
	| 'therapie'
	// Common
	| 'header'
	| 'notes'
	| 'signature';

export type TemplateType = 'soap' | 'swiss';

export interface TranscriptBlock {
	id: string;
	type: BlockType;
	label: string;
	content: string;
}

export interface UploadedFile {
	id: string;
	name: string;
	type: 'audio' | 'image';
	size: number;
	url: string; // Blob URL for preview
	durationSeconds?: number; // For audio
}

export interface Transcript {
	id: string;
	orderId: string;
	// New block-based structure
	outputDocumentType: OutputDocumentType;
	templateUsed: TemplateType;
	blocks: TranscriptBlock[];
	// Legacy field for backwards compatibility
	content?: string; // HTML content (deprecated, use blocks)
	version: number;
	aiGenerated: boolean;
	humanReviewed: boolean;
	reviewerId?: string;
	reviewedAt?: Date;
	reviewNotes?: string;
}

// Helper to convert blocks to HTML content for legacy compatibility
export function blocksToHtml(blocks: TranscriptBlock[]): string {
	return blocks
		.map((block) => `<div class="block block-${block.type}"><h4>${block.label}</h4><p>${block.content.replace(/\n/g, '<br>')}</p></div>`)
		.join('\n');
}

// Helper to generate a new block ID
export function generateBlockId(): string {
	return `block-${crypto.randomUUID().slice(0, 8)}`;
}

export interface Order {
	id: string;
	orderNumber: string;
	customerId: string;
	customerName: string;
	tier: ServiceTier;
	status: OrderStatus;
	documentType: DocumentType;
	patientInitials: string;
	notes?: string;
	priority: 'normal' | 'urgent';
	files: UploadedFile[];
	transcript?: Transcript;
	assignedNurseId?: string;
	createdAt: Date;
	updatedAt: Date;
	estimatedCompletion?: Date;
	completedAt?: Date;
	aiConfidenceScore?: number;
}

// Generate order number
let orderCounter = 1000;
export function generateOrderNumber(): string {
	orderCounter++;
	const date = new Date();
	const year = date.getFullYear();
	const month = String(date.getMonth() + 1).padStart(2, '0');
	return `PM-${year}${month}-${orderCounter}`;
}

// Initial demo orders
const initialOrders: Order[] = [
	{
		id: 'order-1',
		orderNumber: 'PM-202501-1001',
		customerId: 'customer-demo',
		customerName: 'Dr. Schmidt',
		tier: 'light',
		status: 'ready',
		documentType: 'medical_report',
		patientInitials: 'M.K.',
		priority: 'normal',
		files: [
			{
				id: 'file-1',
				name: 'patient_recording.mp3',
				type: 'audio',
				size: 2500000,
				url: '',
				durationSeconds: 180
			}
		],
		transcript: {
			id: 'transcript-1',
			orderId: 'order-1',
			outputDocumentType: 'arztbrief',
			templateUsed: 'swiss',
			blocks: [
				{ id: 'b1-1', type: 'kopfdaten', label: 'Kopfdaten', content: 'Patient: M.K., 52J\nDatum: 20.12.2024\nDokumentart: Arztbrief' },
				{ id: 'b1-2', type: 'anamnese', label: 'Anamnese', content: 'Patient M.K. berichtet über Halsschmerzen und Husten seit 3 Tagen. Leichtes Fieber bis 38.2°C.' },
				{ id: 'b1-3', type: 'befund', label: 'Befund', content: 'Rachen gerötet, Tonsillen leicht geschwollen.\nLunge: Vesikuläratmung beidseits, keine Rasselgeräusche.' },
				{ id: 'b1-4', type: 'diagnose', label: 'Diagnose', content: 'Akute Infektion der oberen Atemwege (J06.9)' },
				{ id: 'b1-5', type: 'therapie', label: 'Therapie', content: '1. Körperliche Schonung\n2. Ausreichend Flüssigkeitszufuhr\n3. Bei Bedarf: Ibuprofen 400mg' }
			],
			version: 1,
			aiGenerated: true,
			humanReviewed: false
		},
		createdAt: new Date(Date.now() - 2 * 24 * 60 * 60 * 1000),
		updatedAt: new Date(Date.now() - 1 * 24 * 60 * 60 * 1000),
		completedAt: new Date(Date.now() - 1 * 24 * 60 * 60 * 1000),
		aiConfidenceScore: 94
	},
	{
		id: 'order-2',
		orderNumber: 'PM-202501-1002',
		customerId: 'customer-demo',
		customerName: 'Dr. Schmidt',
		tier: 'pro',
		status: 'processing',
		documentType: 'consultation_note',
		patientInitials: 'H.W.',
		priority: 'urgent',
		files: [
			{
				id: 'file-2',
				name: 'consultation_notes.jpg',
				type: 'image',
				size: 1200000,
				url: ''
			}
		],
		createdAt: new Date(Date.now() - 4 * 60 * 60 * 1000),
		updatedAt: new Date(Date.now() - 2 * 60 * 60 * 1000)
	},
	{
		id: 'order-3',
		orderNumber: 'PM-202501-1003',
		customerId: 'customer-demo',
		customerName: 'Dr. Schmidt',
		tier: 'light',
		status: 'pending',
		documentType: 'discharge_summary',
		patientInitials: 'S.R.',
		priority: 'normal',
		files: [],
		createdAt: new Date(),
		updatedAt: new Date()
	},
	{
		id: 'order-4',
		orderNumber: 'PM-202501-1004',
		customerId: 'customer-other',
		customerName: 'Dr. Mueller',
		tier: 'pro',
		status: 'ai_complete',
		documentType: 'medical_report',
		patientInitials: 'A.B.',
		priority: 'normal',
		files: [
			{
				id: 'file-4',
				name: 'dictation.m4a',
				type: 'audio',
				size: 3500000,
				url: '',
				durationSeconds: 240
			}
		],
		transcript: {
			id: 'transcript-4',
			orderId: 'order-4',
			outputDocumentType: 'befundbericht',
			templateUsed: 'swiss',
			blocks: [
				{ id: 'b4-1', type: 'kopfdaten', label: 'Kopfdaten', content: 'Patient: A.B., 67J\nDatum: 24.12.2024\nDokumentart: Befundbericht' },
				{ id: 'b4-2', type: 'anamnese', label: 'Anamnese', content: 'Kontrolluntersuchung bei bekannter arterieller Hypertonie.\nPatient fühlt sich insgesamt wohl, keine aktuellen Beschwerden.' },
				{ id: 'b4-3', type: 'befund', label: 'Befund', content: 'RR: 120/80 mmHg\nPuls: 68/min, regelmäßig\nAllgemeinzustand: gut' },
				{ id: 'b4-4', type: 'diagnose', label: 'Diagnose', content: 'Arterielle Hypertonie, gut eingestellt (I10)' },
				{ id: 'b4-5', type: 'therapie', label: 'Therapie', content: 'Medikation unverändert fortführen:\n- Ramipril 5mg 1-0-0\nKontrolle in 3 Monaten' }
			],
			version: 1,
			aiGenerated: true,
			humanReviewed: false
		},
		createdAt: new Date(Date.now() - 6 * 60 * 60 * 1000),
		updatedAt: new Date(Date.now() - 1 * 60 * 60 * 1000),
		aiConfidenceScore: 87
	},
	{
		id: 'order-5',
		orderNumber: 'PM-202501-1005',
		customerId: 'customer-other2',
		customerName: 'Dr. Weber',
		tier: 'pro',
		status: 'ai_complete',
		documentType: 'prescription',
		patientInitials: 'C.D.',
		priority: 'urgent',
		files: [
			{
				id: 'file-5',
				name: 'prescription_audio.wav',
				type: 'audio',
				size: 1800000,
				url: '',
				durationSeconds: 90
			}
		],
		transcript: {
			id: 'transcript-5',
			orderId: 'order-5',
			outputDocumentType: 'arztbrief',
			templateUsed: 'swiss',
			blocks: [
				{ id: 'b5-1', type: 'kopfdaten', label: 'Kopfdaten', content: 'Patient: C.D., 34J\nDatum: 25.12.2024\nDokumentart: Arztbrief / Rezept' },
				{ id: 'b5-2', type: 'anamnese', label: 'Anamnese', content: 'Akute Bronchitis seit 5 Tagen. Produktiver Husten mit gelblichem Auswurf.' },
				{ id: 'b5-3', type: 'befund', label: 'Befund', content: 'Auskultation: Bronchitische Rasselgeräusche beidseits basal.\nTemp: 37.8°C' },
				{ id: 'b5-4', type: 'diagnose', label: 'Diagnose', content: 'Akute Bronchitis (J20.9)' },
				{ id: 'b5-5', type: 'therapie', label: 'Therapie', content: 'Rp.:\nAmoxicillin 500mg\nS: 3x täglich 1 Tablette für 7 Tage\n\nBei Verschlechterung Wiedervorstellung.' }
			],
			version: 1,
			aiGenerated: true,
			humanReviewed: false
		},
		createdAt: new Date(Date.now() - 3 * 60 * 60 * 1000),
		updatedAt: new Date(Date.now() - 30 * 60 * 1000),
		aiConfidenceScore: 92
	}
];

// Orders store
export const orders = writable<Order[]>(initialOrders);

// Derived: orders for current customer
export const customerOrders = derived([orders, currentUser], ([$orders, $user]) =>
	$user ? $orders.filter((o) => o.customerId === $user.id).sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime()) : []
);

// Derived: orders in nurse review queue (pro tier, ai_complete status)
export const nurseQueue = derived([orders, currentUser], ([$orders, $user]) =>
	$user?.role === 'nurse'
		? $orders
				.filter((o) => o.tier === 'pro' && (o.status === 'ai_complete' || o.status === 'in_review'))
				.sort((a, b) => {
					// Urgent first, then by date
					if (a.priority === 'urgent' && b.priority !== 'urgent') return -1;
					if (b.priority === 'urgent' && a.priority !== 'urgent') return 1;
					return a.createdAt.getTime() - b.createdAt.getTime();
				})
		: []
);

// Create new order
export function createOrder(data: {
	tier: ServiceTier;
	documentType: DocumentType;
	patientInitials: string;
	notes?: string;
	priority?: 'normal' | 'urgent';
}): Order {
	const user = get(currentUser);
	if (!user) throw new Error('Not authenticated');

	const order: Order = {
		id: crypto.randomUUID(),
		orderNumber: generateOrderNumber(),
		customerId: user.id,
		customerName: user.displayName,
		tier: data.tier,
		status: 'pending',
		documentType: data.documentType,
		patientInitials: data.patientInitials,
		notes: data.notes,
		priority: data.priority || 'normal',
		files: [],
		createdAt: new Date(),
		updatedAt: new Date()
	};

	orders.update((o) => [order, ...o]);
	return order;
}

// Add file to order
export function addFileToOrder(orderId: string, file: UploadedFile): void {
	orders.update((o) =>
		o.map((order) =>
			order.id === orderId
				? { ...order, files: [...order.files, file], updatedAt: new Date() }
				: order
		)
	);
}

// Remove file from order
export function removeFileFromOrder(orderId: string, fileId: string): void {
	orders.update((o) =>
		o.map((order) =>
			order.id === orderId
				? { ...order, files: order.files.filter((f) => f.id !== fileId), updatedAt: new Date() }
				: order
		)
	);
}

// Submit order for processing
export function submitOrder(orderId: string): void {
	orders.update((o) =>
		o.map((order) =>
			order.id === orderId ? { ...order, status: 'processing', updatedAt: new Date() } : order
		)
	);

	// Simulate AI processing
	setTimeout(() => {
		orders.update((o) =>
			o.map((order) => {
				if (order.id !== orderId) return order;

				const newStatus: OrderStatus = order.tier === 'light' ? 'ready' : 'ai_complete';
				const today = new Date().toLocaleDateString('de-DE');
				return {
					...order,
					status: newStatus,
					updatedAt: new Date(),
					completedAt: order.tier === 'light' ? new Date() : undefined,
					aiConfidenceScore: Math.floor(Math.random() * 15) + 85, // 85-100
					transcript: {
						id: crypto.randomUUID(),
						orderId: order.id,
						outputDocumentType: 'arztbrief',
						templateUsed: 'swiss',
						blocks: [
							{ id: generateBlockId(), type: 'kopfdaten', label: 'Kopfdaten', content: `Patient: ${order.patientInitials}\nDatum: ${today}\nDokumentart: Arztbrief` },
							{ id: generateBlockId(), type: 'anamnese', label: 'Anamnese', content: '[KI-generierte Anamnese wird hier eingefügt...]' },
							{ id: generateBlockId(), type: 'befund', label: 'Befund', content: '[KI-generierte Befunde werden hier eingefügt...]' },
							{ id: generateBlockId(), type: 'diagnose', label: 'Diagnose', content: '[KI-generierte Diagnose wird hier eingefügt...]' },
							{ id: generateBlockId(), type: 'therapie', label: 'Therapie', content: '[KI-generierte Therapieempfehlung wird hier eingefügt...]' }
						],
						version: 1,
						aiGenerated: true,
						humanReviewed: false
					}
				};
			})
		);
	}, 3000); // 3 second mock processing time
}

// Update transcript blocks (nurse review)
export function updateTranscriptBlocks(
	orderId: string,
	blocks: TranscriptBlock[],
	outputDocumentType?: OutputDocumentType,
	templateUsed?: TemplateType,
	reviewNotes?: string
): void {
	orders.update((o) =>
		o.map((order) => {
			if (order.id !== orderId || !order.transcript) return order;
			return {
				...order,
				status: 'in_review',
				updatedAt: new Date(),
				transcript: {
					...order.transcript,
					blocks,
					outputDocumentType: outputDocumentType || order.transcript.outputDocumentType,
					templateUsed: templateUsed || order.transcript.templateUsed,
					version: order.transcript.version + 1,
					reviewNotes
				}
			};
		})
	);
}

// Legacy: Update transcript content (deprecated, use updateTranscriptBlocks)
export function updateTranscript(orderId: string, content: string, reviewNotes?: string): void {
	orders.update((o) =>
		o.map((order) => {
			if (order.id !== orderId || !order.transcript) return order;
			return {
				...order,
				status: 'in_review',
				updatedAt: new Date(),
				transcript: {
					...order.transcript,
					content,
					version: order.transcript.version + 1,
					reviewNotes
				}
			};
		})
	);
}

// Approve transcript (nurse)
export function approveTranscript(orderId: string): void {
	const user = get(currentUser);
	orders.update((o) =>
		o.map((order) => {
			if (order.id !== orderId || !order.transcript) return order;
			return {
				...order,
				status: 'ready',
				updatedAt: new Date(),
				completedAt: new Date(),
				transcript: {
					...order.transcript,
					humanReviewed: true,
					reviewerId: user?.id,
					reviewedAt: new Date()
				}
			};
		})
	);
}

// Mark as downloaded
export function markAsDownloaded(orderId: string): void {
	orders.update((o) =>
		o.map((order) =>
			order.id === orderId ? { ...order, status: 'downloaded', updatedAt: new Date() } : order
		)
	);
}

// Get order by ID
export function getOrderById(orderId: string): Order | undefined {
	let found: Order | undefined;
	orders.subscribe((o) => {
		found = o.find((order) => order.id === orderId);
	})();
	return found;
}
