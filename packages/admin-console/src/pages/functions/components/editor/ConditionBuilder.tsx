// Re-export from shared component for backward compatibility
// The ConditionBuilder has been moved to a shared location to support both
// workflow conditions (fieldPrefix='input.') and permission conditions (fieldPrefix='resource.')
export { ConditionBuilder, type ConditionBuilderProps } from '../../../../components/ConditionBuilder'
