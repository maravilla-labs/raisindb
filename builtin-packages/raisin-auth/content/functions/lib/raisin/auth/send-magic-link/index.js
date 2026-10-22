/**
 * Sends the passwordless sign-in ("magic link") email.
 *
 * The templates below are the JS mirror of MAGIC_LINK_TEMPLATE_TEXT /
 * MAGIC_LINK_TEMPLATE_HTML in crates/raisin-auth/src/jobs/magic_link.rs. They
 * live here, not in Rust, because the wording and branding of a sign-in email
 * are tenant content: editing this node is how a deployment restyles the mail,
 * and doing so requires no server release.
 *
 * The URL arrives fully built. This function must NEVER assemble it from a
 * host or origin it was handed, and must never append the raw token to
 * anything else: the server built it from the tenant's configured base_url
 * precisely so that an attacker-controlled header cannot redirect a one-time
 * token to their own host.
 *
 * @param {Object} input - The function input
 * @param {string} input.email - Address the sign-in link is sent to
 * @param {string} input.magic_link_url - Fully built verification URL
 * @param {number} input.expires_in_minutes - Link lifetime, shown in the body
 * @param {string} [input.template] - Optional template name; "password_reset" sends the reset wording
 * @param {string} [input.subject] - Tenant-written subject (overrides the built-in)
 * @param {string} [input.text] - Tenant-written, pre-rendered plain-text body
 * @param {string} [input.html] - Tenant-written, pre-rendered HTML body
 * @returns {Promise<{sent: boolean, message_id?: string, provider?: string}>}
 */

const TEMPLATE_TEXT = `
Hello,

You requested a magic link to sign in to your account.

Click here to sign in:
{magic_link_url}

This link will expire in {expires_in_minutes} minutes.

If you didn't request this link, you can safely ignore this email.

Best regards,
The Team
`;

const TEMPLATE_HTML = `
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Sign In</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h1 style="color: #2563eb;">Sign In</h1>
        <p>Hello,</p>
        <p>You requested a magic link to sign in to your account.</p>
        <p>
            <a href="{magic_link_url}"
               style="display: inline-block; background-color: #2563eb; color: white;
                      padding: 12px 24px; text-decoration: none; border-radius: 6px;
                      font-weight: bold;">
                Sign In
            </a>
        </p>
        <p style="color: #666; font-size: 14px;">
            This link will expire in {expires_in_minutes} minutes.
        </p>
        <p style="color: #999; font-size: 12px; margin-top: 30px;">
            If you didn't request this link, you can safely ignore this email.
        </p>
    </div>
</body>
</html>
`;

const RESET_TEXT = `
Hello,

We received a request to reset the password for your account.

Click here to choose a new password:
{magic_link_url}

This link will expire in {expires_in_minutes} minutes and can only be used once.

If you didn't request this, you can safely ignore this email - your password
stays as it is.

Best regards,
The Team
`;

const RESET_HTML = `
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Reset your password</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h1 style="color: #2563eb;">Reset your password</h1>
        <p>Hello,</p>
        <p>We received a request to reset the password for your account.</p>
        <p>
            <a href="{magic_link_url}"
               style="display: inline-block; background-color: #2563eb; color: white;
                      padding: 12px 24px; text-decoration: none; border-radius: 6px;
                      font-weight: bold;">
                Choose a new password
            </a>
        </p>
        <p style="color: #666; font-size: 14px;">
            This link will expire in {expires_in_minutes} minutes and can only be used once.
        </p>
        <p style="color: #999; font-size: 12px; margin-top: 30px;">
            If you didn't request this, you can safely ignore this email &mdash;
            your password stays as it is.
        </p>
    </div>
</body>
</html>
`;

function render(template, values) {
    return Object.keys(values).reduce(
        (out, key) => out.split(`{${key}}`).join(values[key]),
        template
    );
}

async function sendMagicLink(input) {
    const { email, magic_link_url, expires_in_minutes } = input;

    if (!email || !magic_link_url) {
        throw new Error("email and magic_link_url are required");
    }

    const values = {
        magic_link_url: magic_link_url,
        expires_in_minutes: String(expires_in_minutes || 15)
    };

    // The sender identity is NOT set here: raisin.email.send takes it from the
    // tenant's /config/email node, so a change to this function cannot make
    // mail go out as an address the tenant never verified.
    const isReset = input.template === "password_reset";
    // `subject` / `text` / `html` arrive already rendered when the tenant wrote
    // its own wording on the Email settings page; the built-in text is only the
    // fallback for a tenant that has not.
    const receipt = await raisin.email.send({
        to: email,
        subject: input.subject || (isReset ? "Reset your password" : "Your sign-in link"),
        text: input.text || render(isReset ? RESET_TEXT : TEMPLATE_TEXT, values),
        html: input.html || render(isReset ? RESET_HTML : TEMPLATE_HTML, values)
    });

    return {
        sent: true,
        message_id: receipt.message_id,
        provider: receipt.provider
    };
}

// Export for RaisinDB function runtime
module.exports = { sendMagicLink };
