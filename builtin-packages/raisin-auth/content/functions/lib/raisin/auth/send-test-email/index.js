/**
 * Sends one test message through a configured email provider.
 *
 * Nothing about this function is special-cased on the server: it calls
 * `raisin.email.send` exactly as any tenant function would. That is the whole
 * point — a test that took a shortcut would prove the shortcut works.
 *
 * @param {Object} input
 * @param {string} input.to - Address to send the test to
 * @param {string} [input.provider] - Configured provider name; omitted tests
 *   the tenant's default sender, which is the one magic-link sign-in uses.
 * @returns {Promise<{sent: boolean, message_id: string, provider: string, sender: string}>}
 */
async function sendTestEmail(input) {
    const to = (input && input.to ? String(input.to) : "").trim();
    if (!to || to.indexOf("@") === -1) {
        throw new Error("`to` must be an email address");
    }

    // Trimmed and dropped when empty: an unset field in the console arrives as
    // "" and must mean "the default", not a provider named "".
    const provider = (input && input.provider ? String(input.provider) : "").trim();

    const when = new Date().toISOString();
    const label = provider || "default";

    const message = {
        to: to,
        subject: "RaisinDB test message",
        text: [
            "This is a test message from RaisinDB.",
            "",
            "If you are reading it, this provider can deliver mail:",
            "  provider: " + label,
            "  sent at:  " + when,
            "",
            "Sign-in links and notifications use the same path.",
        ].join("\n"),
        html:
            '<div style="font-family: system-ui, sans-serif; line-height: 1.6">' +
            "<h2 style=\"margin:0 0 12px\">RaisinDB test message</h2>" +
            "<p>If you are reading this, this provider can deliver mail.</p>" +
            '<table style="font-size:14px; border-collapse:collapse">' +
            "<tr><td style=\"padding-right:12px; color:#666\">provider</td><td><code>" +
            label +
            "</code></td></tr>" +
            "<tr><td style=\"padding-right:12px; color:#666\">sent at</td><td><code>" +
            when +
            "</code></td></tr>" +
            "</table>" +
            '<p style="color:#666; font-size:13px">Sign-in links and notifications use the same path.</p>' +
            "</div>",
    };
    if (provider) {
        message.provider = provider;
    }

    const receipt = await raisin.email.send(message);

    return {
        sent: true,
        message_id: receipt.message_id,
        provider: receipt.provider,
        sender: receipt.sender,
    };
}

module.exports = { sendTestEmail };
