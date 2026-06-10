<script setup lang="ts">
/**
 * Login form bound to useAuth().login (client.loginWithEmail under the
 * hood: HTTP login + WebSocket connect + JWT auth in one call).
 * Demo credentials are prefilled.
 */
import { ref } from 'vue';
import type { UseAuthReturn } from '@raisindb/client/vue';
import { REPOSITORY } from '../lib/raisin';

const props = defineProps<{ auth: UseAuthReturn }>();

const email = ref('planner@example.com');
const password = ref('Planner12345!');
const error = ref<string | null>(null);
const submitting = ref(false);

async function submit() {
  if (submitting.value) return;
  error.value = null;
  submitting.value = true;
  try {
    await props.auth.login(email.value.trim(), password.value, REPOSITORY);
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="login-screen">
    <form class="login-card" @submit.prevent="submit">
      <h1>Vue Board</h1>
      <p class="login-hint">
        RaisinDB Vue composables demo — sign in with the prefilled demo
        account (<code>planner@example.com</code>).
      </p>
      <label>
        Email
        <input v-model="email" type="email" name="email" autocomplete="username" required />
      </label>
      <label>
        Password
        <input v-model="password" type="password" name="password" autocomplete="current-password" required />
      </label>
      <p v-if="error" class="login-error" role="alert">{{ error }}</p>
      <button type="submit" :disabled="submitting || auth.isLoading.value">
        {{ submitting ? 'Signing in…' : 'Sign in' }}
      </button>
    </form>
  </div>
</template>
