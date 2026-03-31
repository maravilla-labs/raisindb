<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import {
		Stethoscope,
		FileAudio,
		ClipboardCheck,
		Clock,
		Shield,
		ArrowRight,
		Upload,
		Mic,
		FileText,
		Target,
		Lock,
		Users,
		CheckCircle
	} from 'lucide-svelte';
	import { Button } from '$lib/components/shared';
	import { currentUser, restoreSession } from '$lib/stores/auth';
	import heroImage from '$lib/assets/hero.jpeg';
	import janineImage from '$lib/assets/janine-nobgtrasnaprentbg.png';

	onMount(() => {
		restoreSession();
	});

	// Redirect authenticated users
	$effect(() => {
		if ($currentUser) {
			goto($currentUser.role === 'nurse' ? '/nurse/dashboard' : '/dashboard');
		}
	});

	const features = [
		{
			icon: FileAudio,
			title: 'Audio & Bilder',
			description: 'Laden Sie Diktate oder handschriftliche Notizen zur Transkription hoch.'
		},
		{
			icon: ClipboardCheck,
			title: 'Manuelle Prüfung',
			description: 'Qualifizierte Pflegekräfte prüfen KI-Transkriptionen für Pro-Tier Genauigkeit.'
		},
		{
			icon: Clock,
			title: 'Schnelle Bearbeitung',
			description: 'Light-Tier in 24 Stunden, Pro-Tier mit Prüfung in 48 Stunden.'
		},
		{
			icon: Shield,
			title: 'DSGVO-konform',
			description: 'Ihre Patientendaten sind sicher und werden innerhalb der EU verarbeitet.'
		}
	];

	const steps = [
		{
			icon: Upload,
			title: 'Hochladen',
			description: 'Audio oder Notizen sicher hochladen'
		},
		{
			icon: Mic,
			title: 'Transkription',
			description: 'KI + manuelle Überprüfung'
		},
		{
			icon: FileText,
			title: 'Abrufen',
			description: 'Fertige Dokumente herunterladen'
		}
	];
</script>

<svelte:head>
	<title>Pocket Medico - Hybride KI-Transkription für Ärzte</title>
</svelte:head>

<div class="min-h-screen bg-gradient-to-b from-blue-50 to-white">
	<!-- Header -->
	<header class="fixed left-0 right-0 top-0 z-50 border-b border-gray-200/50 bg-white/70 backdrop-blur-md">
		<nav class="mx-auto flex max-w-6xl items-center justify-between px-6 py-4">
			<div class="flex items-center gap-2">
				<Stethoscope class="h-8 w-8 text-blue-600" />
				<span class="text-xl font-bold text-gray-900">Pocket Medico</span>
			</div>
			<div class="flex items-center gap-3">
				<a href="/login">
					<Button variant="ghost">Anmelden</Button>
				</a>
				<a href="/register">
					<Button>Jetzt starten</Button>
				</a>
			</div>
		</nav>
	</header>

	<!-- Hero -->
	<section class="mx-auto max-w-6xl px-6 pb-20 pt-32">
		<div class="grid items-center gap-12 lg:grid-cols-2">
			<div class="text-center lg:text-left">
				<div class="inline-flex items-center gap-2 rounded-full bg-blue-100 px-4 py-1.5 text-sm font-medium text-blue-700">
					<span class="relative flex h-2 w-2">
						<span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-blue-400 opacity-75"></span>
						<span class="relative inline-flex h-2 w-2 rounded-full bg-blue-500"></span>
					</span>
					Jetzt verfügbar in Deutschland & Österreich
				</div>

				<h1 class="mt-8 text-5xl font-bold tracking-tight text-gray-900 sm:text-6xl">
					Hybride KI-Transkription
					<br />
					<span class="text-blue-600">für Ärzte</span>
				</h1>

				<p class="mt-6 text-lg text-gray-600">
					Genauigkeit, der Sie vertrauen können. Geschwindigkeit, die Sie brauchen.
					Laden Sie Audiodateien oder Notizen hoch und erhalten Sie präzise, professionell transkribierte Dokumente.
				</p>

				<div class="mt-10 flex flex-col items-center gap-4 sm:flex-row lg:justify-start">
					<a href="/register">
						<Button class="px-8 py-3 text-lg">
							Kostenlos testen
							<ArrowRight class="ml-2 h-5 w-5" />
						</Button>
					</a>
					<a href="/login" class="text-sm font-medium text-gray-600 hover:text-gray-900">
						Bereits registriert? Anmelden
					</a>
				</div>
			</div>

			<div class="relative">
				<img
					src={heroImage}
					alt="Medizinische Transkription"
					class="rounded-2xl shadow-2xl"
				/>
			</div>
		</div>
	</section>

	<!-- How it works -->
	<section class="border-y border-gray-200 bg-white py-16">
		<div class="mx-auto max-w-6xl px-6">
			<h2 class="text-center text-2xl font-bold text-gray-900">So einfach funktioniert es</h2>
			<div class="mt-10 grid gap-8 sm:grid-cols-3">
				{#each steps as step, index}
					<div class="text-center">
						<div class="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-blue-100">
							<step.icon class="h-7 w-7 text-blue-600" />
						</div>
						<div class="mt-2 text-sm font-bold text-blue-600">Schritt {index + 1}</div>
						<h3 class="mt-2 font-semibold text-gray-900">{step.title}</h3>
						<p class="mt-1 text-sm text-gray-600">{step.description}</p>
					</div>
				{/each}
			</div>
		</div>
	</section>

	<!-- Features -->
	<section class="mx-auto max-w-6xl px-6 py-20">
		<h2 class="text-center text-3xl font-bold text-gray-900">Ihre Vorteile</h2>
		<p class="mx-auto mt-4 max-w-xl text-center text-gray-600">
			Warum Pocket Medico die beste Wahl für Ihre medizinischen Transkriptionen ist
		</p>

		<div class="mt-12 grid gap-6 sm:grid-cols-2 lg:grid-cols-4">
			{#each features as feature}
				<div class="rounded-xl border border-gray-200 bg-white p-6 shadow-sm">
					<div class="flex h-12 w-12 items-center justify-center rounded-lg bg-blue-100">
						<feature.icon class="h-6 w-6 text-blue-600" />
					</div>
					<h3 class="mt-4 font-semibold text-gray-900">{feature.title}</h3>
					<p class="mt-2 text-sm text-gray-600">{feature.description}</p>
				</div>
			{/each}
		</div>
	</section>

	<!-- Pricing Preview -->
	<section class="border-t border-gray-200 bg-gray-50 py-16">
		<div class="mx-auto max-w-6xl px-6">
			<h2 class="text-center text-3xl font-bold text-gray-900">Einfache Preisgestaltung</h2>
			<p class="mx-auto mt-4 max-w-xl text-center text-gray-600">
				Wählen Sie die passende Lösung für Ihre Praxis. Keine versteckten Kosten.
			</p>

			<div class="mt-10 grid gap-6 lg:grid-cols-2">
				<!-- Light Tier -->
				<div class="rounded-xl border border-gray-200 bg-white p-8">
					<h3 class="text-xl font-semibold text-gray-900">Light</h3>
					<p class="mt-2 text-sm text-gray-600">KI-Transkription für einfache Notizen</p>
					<p class="mt-4">
						<span class="text-4xl font-bold text-gray-900">€0,10</span>
						<span class="text-gray-500">/Minute</span>
					</p>
					<ul class="mt-6 space-y-3 text-sm text-gray-600">
						<li class="flex items-center gap-2">
							<CheckCircle class="h-5 w-5 text-green-500" />
							KI-gestützte Transkription
						</li>
						<li class="flex items-center gap-2">
							<CheckCircle class="h-5 w-5 text-green-500" />
							24-Stunden Lieferung
						</li>
						<li class="flex items-center gap-2">
							<CheckCircle class="h-5 w-5 text-green-500" />
							Standardformatierung
						</li>
					</ul>
				</div>

				<!-- Pro Tier -->
				<div class="relative rounded-xl border-2 border-blue-600 bg-white p-8">
					<span class="absolute -top-3 left-6 rounded-full bg-blue-600 px-3 py-1 text-xs font-medium text-white">
						Empfohlen
					</span>
					<h3 class="text-xl font-semibold text-gray-900">Pro</h3>
					<p class="mt-2 text-sm text-gray-600">Mit manueller Überprüfung für höchste Genauigkeit</p>
					<p class="mt-4">
						<span class="text-4xl font-bold text-gray-900">€0,25</span>
						<span class="text-gray-500">/Minute</span>
					</p>
					<ul class="mt-6 space-y-3 text-sm text-gray-600">
						<li class="flex items-center gap-2">
							<CheckCircle class="h-5 w-5 text-green-500" />
							KI + manuelle Überprüfung
						</li>
						<li class="flex items-center gap-2">
							<CheckCircle class="h-5 w-5 text-green-500" />
							48-Stunden Lieferung
						</li>
						<li class="flex items-center gap-2">
							<CheckCircle class="h-5 w-5 text-green-500" />
							Medizinische Terminologie-Prüfung
						</li>
						<li class="flex items-center gap-2">
							<CheckCircle class="h-5 w-5 text-green-500" />
							Prioritäts-Support
						</li>
					</ul>
				</div>
			</div>
		</div>
	</section>

	<!-- Team -->
	<section class="py-16">
		<div class="mx-auto max-w-6xl px-6">
			<div class="grid items-center gap-12 lg:grid-cols-2">
				<div class="order-2 lg:order-1">
					<h2 class="text-3xl font-bold text-gray-900">Erfahrung & Innovation</h2>
					<p class="mt-4 text-gray-600">
						Mit Janine Taş, Mitgründerin der SOLUTAS GmbH und Pionierin in der Telemedizin,
						garantieren wir höchste Qualität. Unser hybrides Modell kombiniert KI-Geschwindigkeit
						mit menschlicher Expertise für zuverlässige Ergebnisse.
					</p>
					<a href="/register" class="mt-6 inline-block">
						<Button>
							Mehr erfahren
							<ArrowRight class="ml-2 h-4 w-4" />
						</Button>
					</a>
				</div>
				<div class="order-1 lg:order-2">
					<img
						src={janineImage}
						alt="Janine Taş - Mitgründerin"
						class="mx-auto max-h-[400px] object-contain"
					/>
				</div>
			</div>
		</div>
	</section>

	<!-- Demo credentials -->
	<section class="border-t border-gray-200 bg-blue-600 py-12">
		<div class="mx-auto max-w-3xl px-6 text-center">
			<h2 class="text-2xl font-bold text-white">Jetzt kostenlos testen</h2>
			<p class="mt-2 text-blue-100">
				Lernen Sie Pocket Medico kennen mit unseren Demo-Zugängen
			</p>

			<div class="mx-auto mt-8 max-w-md rounded-xl bg-white p-6 text-left shadow-lg">
				<p class="text-sm font-medium text-gray-900">Demo-Zugänge</p>
				<div class="mt-3 space-y-2 text-sm text-gray-600">
					<div class="flex justify-between">
						<span>Arzt:</span>
						<code class="rounded bg-gray-100 px-2 py-0.5 text-xs">doctor@demo.com / demo1234</code>
					</div>
					<div class="flex justify-between">
						<span>Pflegekraft:</span>
						<code class="rounded bg-gray-100 px-2 py-0.5 text-xs">nurse@demo.com / demo1234</code>
					</div>
				</div>
				<div class="mt-6 flex gap-3">
					<a href="/login" class="flex-1">
						<Button variant="secondary" class="w-full">Anmelden</Button>
					</a>
					<a href="/register" class="flex-1">
						<Button class="w-full">Registrieren</Button>
					</a>
				</div>
			</div>
		</div>
	</section>

	<!-- Footer -->
	<footer class="border-t border-gray-200 bg-white py-8">
		<div class="mx-auto max-w-6xl px-6 text-center text-sm text-gray-500">
			<p>© 2024 Pocket Medico. Alle Rechte vorbehalten.</p>
			<p class="mt-2">Mit Sorgfalt entwickelt für medizinische Fachkräfte.</p>
		</div>
	</footer>
</div>
