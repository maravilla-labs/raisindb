import type { BlockType, TemplateType, OutputDocumentType, TranscriptBlock } from '$lib/stores/orders';
import { generateBlockId } from '$lib/stores/orders';

export interface BlockDefinition {
	type: BlockType;
	label: string;
	placeholder: string;
	required: boolean;
}

export interface Template {
	id: TemplateType;
	name: string;
	description: string;
	blocks: BlockDefinition[];
}

// Block styling configuration
export const blockStyles: Record<BlockType, { bg: string; border: string; headerBg: string }> = {
	// SOAP Format
	subjective: { bg: 'bg-blue-50', border: 'border-blue-300', headerBg: 'bg-blue-100' },
	objective: { bg: 'bg-green-50', border: 'border-green-300', headerBg: 'bg-green-100' },
	assessment: { bg: 'bg-yellow-50', border: 'border-yellow-300', headerBg: 'bg-yellow-100' },
	plan: { bg: 'bg-purple-50', border: 'border-purple-300', headerBg: 'bg-purple-100' },
	// Swiss Format
	kopfdaten: { bg: 'bg-gray-50', border: 'border-gray-300', headerBg: 'bg-gray-100' },
	anamnese: { bg: 'bg-blue-50', border: 'border-blue-300', headerBg: 'bg-blue-100' },
	befund: { bg: 'bg-green-50', border: 'border-green-300', headerBg: 'bg-green-100' },
	diagnose: { bg: 'bg-yellow-50', border: 'border-yellow-300', headerBg: 'bg-yellow-100' },
	therapie: { bg: 'bg-purple-50', border: 'border-purple-300', headerBg: 'bg-purple-100' },
	// Common
	header: { bg: 'bg-gray-50', border: 'border-gray-300', headerBg: 'bg-gray-100' },
	notes: { bg: 'bg-gray-50', border: 'border-gray-200', headerBg: 'bg-gray-100' },
	signature: { bg: 'bg-slate-50', border: 'border-slate-300', headerBg: 'bg-slate-100' }
};

// Swiss medical template (German standard)
export const swissTemplate: Template = {
	id: 'swiss',
	name: 'Schweizer Format',
	description: 'Standardformat für deutschsprachige medizinische Dokumentation',
	blocks: [
		{
			type: 'kopfdaten',
			label: 'Kopfdaten',
			placeholder: 'Patient: [Name, Alter]\nDatum: [Datum]\nDokumentart: [Typ]',
			required: true
		},
		{
			type: 'anamnese',
			label: 'Anamnese',
			placeholder: 'Eigenanamnese, Fremdanamnese, aktuelle Beschwerden...',
			required: true
		},
		{
			type: 'befund',
			label: 'Befund',
			placeholder: 'Körperliche Untersuchung, Vitalzeichen, klinische Befunde...',
			required: true
		},
		{
			type: 'diagnose',
			label: 'Diagnose',
			placeholder: 'Haupt- und Nebendiagnosen mit ICD-Codes...',
			required: true
		},
		{
			type: 'therapie',
			label: 'Therapie',
			placeholder: 'Medikation, Empfehlungen, weiteres Vorgehen...',
			required: true
		},
		{
			type: 'notes',
			label: 'Bemerkungen',
			placeholder: 'Zusätzliche Hinweise...',
			required: false
		},
		{
			type: 'signature',
			label: 'Unterschrift',
			placeholder: 'Name, Datum, Funktion',
			required: false
		}
	]
};

// SOAP template (International standard)
export const soapTemplate: Template = {
	id: 'soap',
	name: 'SOAP Format',
	description: 'International standard medical documentation format',
	blocks: [
		{
			type: 'header',
			label: 'Header',
			placeholder: 'Patient: [Name, Age]\nDate: [Date]\nDocument Type: [Type]',
			required: true
		},
		{
			type: 'subjective',
			label: 'Subjective',
			placeholder: "Patient's reported symptoms, history, complaints...",
			required: true
		},
		{
			type: 'objective',
			label: 'Objective',
			placeholder: 'Physical examination findings, vital signs, test results...',
			required: true
		},
		{
			type: 'assessment',
			label: 'Assessment',
			placeholder: 'Diagnosis, clinical impression, differential diagnoses...',
			required: true
		},
		{
			type: 'plan',
			label: 'Plan',
			placeholder: 'Treatment plan, medications, follow-up...',
			required: true
		},
		{
			type: 'notes',
			label: 'Notes',
			placeholder: 'Additional notes...',
			required: false
		},
		{
			type: 'signature',
			label: 'Signature',
			placeholder: 'Name, Date, Title',
			required: false
		}
	]
};

export const templates: Record<TemplateType, Template> = {
	swiss: swissTemplate,
	soap: soapTemplate
};

// Output document type labels (German)
export const documentTypeLabels: Record<OutputDocumentType, string> = {
	arztbrief: 'Arztbrief',
	befundbericht: 'Befundbericht',
	entlassungsbericht: 'Entlassungsbericht',
	konsiliarbericht: 'Konsiliarbericht',
	ueberweisungsbrief: 'Überweisungsbrief'
};

// All available block types for adding
export const availableBlockTypes: { type: BlockType; label: string }[] = [
	// Swiss blocks
	{ type: 'kopfdaten', label: 'Kopfdaten' },
	{ type: 'anamnese', label: 'Anamnese' },
	{ type: 'befund', label: 'Befund' },
	{ type: 'diagnose', label: 'Diagnose' },
	{ type: 'therapie', label: 'Therapie' },
	// SOAP blocks
	{ type: 'subjective', label: 'Subjective' },
	{ type: 'objective', label: 'Objective' },
	{ type: 'assessment', label: 'Assessment' },
	{ type: 'plan', label: 'Plan' },
	// Common blocks
	{ type: 'header', label: 'Header' },
	{ type: 'notes', label: 'Bemerkungen / Notes' },
	{ type: 'signature', label: 'Unterschrift / Signature' }
];

// Helper to create blocks from a template
export function createBlocksFromTemplate(template: Template): TranscriptBlock[] {
	return template.blocks.map((def) => ({
		id: generateBlockId(),
		type: def.type,
		label: def.label,
		content: ''
	}));
}

// Helper to get block label by type
export function getBlockLabel(type: BlockType): string {
	const found = availableBlockTypes.find((b) => b.type === type);
	return found?.label || type;
}

// Helper to create a new empty block
export function createEmptyBlock(type: BlockType): TranscriptBlock {
	return {
		id: generateBlockId(),
		type,
		label: getBlockLabel(type),
		content: ''
	};
}
