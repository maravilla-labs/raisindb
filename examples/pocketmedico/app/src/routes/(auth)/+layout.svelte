<script lang="ts">
	import '../../app.css';
	import { Stethoscope } from 'lucide-svelte';
	import { Toast } from '$lib/components/layout';

	let { children } = $props();
</script>

<div class="flex min-h-screen flex-col items-center justify-center bg-gray-50 px-4 py-12">
	<!-- Logo -->
	<div class="mb-8 flex items-center gap-3">
		<div class="flex h-12 w-12 items-center justify-center rounded-xl bg-blue-500">
			<Stethoscope class="h-7 w-7 text-white" />
		</div>
		<div>
			<div class="text-xl font-bold text-gray-900">Pocket Medico</div>
			<div class="text-sm text-gray-500">Medical Transcription</div>
		</div>
	</div>

	<!-- Auth Card -->
	<div class="w-full max-w-md rounded-xl bg-white p-8 shadow-sm border border-gray-200">
		{@render children()}
	</div>

	<!-- Demo Credentials -->
	<div class="mt-6 rounded-lg bg-blue-50 p-4 text-center">
		<p class="text-sm font-medium text-blue-900">Demo Credentials</p>
		<p class="mt-1 text-xs text-blue-700">
			Doctor: doctor@demo.com / demo1234
		</p>
		<p class="text-xs text-blue-700">
			Nurse: nurse@demo.com / demo1234
		</p>
	</div>
</div>

<Toast />
