/**
 * Demo identity users created by `ci.sh` / `setup.mjs` (see README "Setup").
 * Shared by the login screen (autofill) and the header (quick user switch).
 */
export interface DemoAccount {
  label: string;
  email: string;
  password: string;
}

export const DEMO_ACCOUNTS: DemoAccount[] = [
  { label: 'Planner (manager)', email: 'planner@example.com', password: 'Planner12345!' },
  { label: 'Anna (barista)', email: 'anna@example.com', password: 'Staff12345!' },
  { label: 'Cara (all-round)', email: 'cara@example.com', password: 'Staff12345!' },
];
