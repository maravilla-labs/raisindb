<script lang="ts">
	import { Plus, Save, CheckCircle, FileText, LayoutTemplate } from 'lucide-svelte';
	import type { TranscriptBlock as BlockData, OutputDocumentType, TemplateType } from '$lib/stores/orders';
	import { generateBlockId } from '$lib/stores/orders';
	import {
		templates,
		documentTypeLabels,
		availableBlockTypes,
		createBlocksFromTemplate,
		getBlockLabel
	} from '$lib/config/transcriptTemplates';
	import TranscriptBlock from './TranscriptBlock.svelte';
	import { Button } from '$lib/components/shared';

	interface Props {
		blocks: BlockData[];
		outputDocumentType: OutputDocumentType;
		templateUsed: TemplateType;
		onBlocksChange?: (blocks: BlockData[]) => void;
		onDocumentTypeChange?: (type: OutputDocumentType) => void;
		onTemplateChange?: (template: TemplateType) => void;
		onSave?: () => void;
		onApprove?: () => void;
		saving?: boolean;
		approving?: boolean;
	}

	let {
		blocks,
		outputDocumentType,
		templateUsed,
		onBlocksChange,
		onDocumentTypeChange,
		onTemplateChange,
		onSave,
		onApprove,
		saving = false,
		approving = false
	}: Props = $props();

	let showAddMenu = $state(false);
	let showTemplateConfirm = $state(false);
	let pendingTemplate: TemplateType | null = $state(null);

	const documentTypes: { value: OutputDocumentType; label: string }[] = [
		{ value: 'arztbrief', label: 'Arztbrief' },
		{ value: 'befundbericht', label: 'Befundbericht' },
		{ value: 'entlassungsbericht', label: 'Entlassungsbericht' },
		{ value: 'konsiliarbericht', label: 'Konsiliarbericht' },
		{ value: 'ueberweisungsbrief', label: 'Überweisungsbrief' }
	];

	function updateBlock(index: number, content: string) {
		const newBlocks = [...blocks];
		newBlocks[index] = { ...newBlocks[index], content };
		onBlocksChange?.(newBlocks);
	}

	function deleteBlock(index: number) {
		const newBlocks = blocks.filter((_, i) => i !== index);
		onBlocksChange?.(newBlocks);
	}

	function moveBlock(index: number, direction: 'up' | 'down') {
		const newBlocks = [...blocks];
		const targetIndex = direction === 'up' ? index - 1 : index + 1;
		if (targetIndex < 0 || targetIndex >= newBlocks.length) return;
		[newBlocks[index], newBlocks[targetIndex]] = [newBlocks[targetIndex], newBlocks[index]];
		onBlocksChange?.(newBlocks);
	}

	function addBlock(type: string) {
		const blockType = type as BlockData['type'];
		const newBlock: BlockData = {
			id: generateBlockId(),
			type: blockType,
			label: getBlockLabel(blockType),
			content: ''
		};
		onBlocksChange?.([...blocks, newBlock]);
		showAddMenu = false;
	}

	function handleTemplateSelect(template: TemplateType) {
		if (blocks.length > 0 && blocks.some((b) => b.content.trim())) {
			// Has content, confirm before replacing
			pendingTemplate = template;
			showTemplateConfirm = true;
		} else {
			applyTemplate(template);
		}
	}

	function applyTemplate(template: TemplateType) {
		const templateDef = templates[template];
		const newBlocks = createBlocksFromTemplate(templateDef);
		onBlocksChange?.(newBlocks);
		onTemplateChange?.(template);
		showTemplateConfirm = false;
		pendingTemplate = null;
	}

	function cancelTemplateChange() {
		showTemplateConfirm = false;
		pendingTemplate = null;
	}
</script>

<div class="flex flex-col gap-4">
	<!-- Toolbar -->
	<div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-gray-200 bg-white p-3">
		<div class="flex flex-wrap items-center gap-3">
			<!-- Document Type Selector -->
			<div class="flex items-center gap-2">
				<FileText class="h-4 w-4 text-gray-400" />
				<select
					value={outputDocumentType}
					onchange={(e) => onDocumentTypeChange?.(e.currentTarget.value as OutputDocumentType)}
					class="rounded border-gray-300 py-1 pl-2 pr-8 text-sm focus:border-blue-500 focus:ring-blue-500"
				>
					{#each documentTypes as dt}
						<option value={dt.value}>{dt.label}</option>
					{/each}
				</select>
			</div>

			<!-- Template Selector -->
			<div class="flex items-center gap-2">
				<LayoutTemplate class="h-4 w-4 text-gray-400" />
				<select
					value={templateUsed}
					onchange={(e) => handleTemplateSelect(e.currentTarget.value as TemplateType)}
					class="rounded border-gray-300 py-1 pl-2 pr-8 text-sm focus:border-blue-500 focus:ring-blue-500"
				>
					<option value="swiss">Schweizer Format</option>
					<option value="soap">SOAP Format</option>
				</select>
			</div>

			<!-- Add Block -->
			<div class="relative">
				<button
					type="button"
					onclick={() => (showAddMenu = !showAddMenu)}
					class="inline-flex items-center gap-1.5 rounded-lg border border-gray-300 bg-white px-3 py-1.5 text-sm font-medium text-gray-700 transition-colors hover:bg-gray-50"
				>
					<Plus class="h-4 w-4" />
					Block hinzufügen
				</button>

				{#if showAddMenu}
					<div class="absolute left-0 top-full z-10 mt-1 w-48 rounded-lg border border-gray-200 bg-white py-1 shadow-lg">
						{#each availableBlockTypes as bt}
							<button
								type="button"
								onclick={() => addBlock(bt.type)}
								class="block w-full px-4 py-2 text-left text-sm text-gray-700 hover:bg-gray-100"
							>
								{bt.label}
							</button>
						{/each}
					</div>
				{/if}
			</div>
		</div>

		<!-- Actions -->
		<div class="flex items-center gap-2">
			<Button variant="secondary" onclick={onSave} loading={saving} disabled={saving || approving}>
				<Save class="mr-1.5 h-4 w-4" />
				Speichern
			</Button>
			<Button onclick={onApprove} loading={approving} disabled={saving || approving}>
				<CheckCircle class="mr-1.5 h-4 w-4" />
				Freigeben
			</Button>
		</div>
	</div>

	<!-- Blocks -->
	<div class="space-y-3">
		{#if blocks.length === 0}
			<div class="rounded-lg border-2 border-dashed border-gray-300 p-8 text-center">
				<FileText class="mx-auto h-12 w-12 text-gray-400" />
				<p class="mt-2 text-sm text-gray-500">Keine Blöcke vorhanden</p>
				<p class="text-xs text-gray-400">
					Wählen Sie eine Vorlage oder fügen Sie Blöcke manuell hinzu
				</p>
			</div>
		{:else}
			{#each blocks as block, index (block.id)}
				<TranscriptBlock
					{block}
					canMoveUp={index > 0}
					canMoveDown={index < blocks.length - 1}
					onUpdate={(content) => updateBlock(index, content)}
					onDelete={() => deleteBlock(index)}
					onMoveUp={() => moveBlock(index, 'up')}
					onMoveDown={() => moveBlock(index, 'down')}
				/>
			{/each}
		{/if}
	</div>
</div>

<!-- Template Confirm Modal -->
{#if showTemplateConfirm}
	<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
		<div class="mx-4 max-w-md rounded-lg bg-white p-6 shadow-xl">
			<h3 class="text-lg font-semibold text-gray-900">Vorlage anwenden?</h3>
			<p class="mt-2 text-sm text-gray-600">
				Das Anwenden einer neuen Vorlage ersetzt alle vorhandenen Blöcke und deren Inhalte. Diese
				Aktion kann nicht rückgängig gemacht werden.
			</p>
			<div class="mt-4 flex justify-end gap-3">
				<Button variant="secondary" onclick={cancelTemplateChange}>Abbrechen</Button>
				<Button onclick={() => pendingTemplate && applyTemplate(pendingTemplate)}>
					Vorlage anwenden
				</Button>
			</div>
		</div>
	</div>
{/if}

<!-- Click outside handler for add menu -->
{#if showAddMenu}
	<button
		type="button"
		class="fixed inset-0 z-0"
		onclick={() => (showAddMenu = false)}
		onkeydown={(e) => e.key === 'Escape' && (showAddMenu = false)}
	></button>
{/if}
