/**
 * ask-user — ask the person a question and wait for the answer.
 *
 * Under an agent run this is a DOMAIN tool: the run reducer answers it (it opens an `ask_user` request and hands the
 * person's answer back as the tool result), so this function body is never the path that runs. It exists so the tool
 * can be listed in an agent's `tools` like any other and so its description and schema reach the model.
 *
 * Execution mode: inline
 * Category: interaction
 */
async function handler() {
  throw new Error('ask-user works only under an agent run: the run waits for the answer.');
}
