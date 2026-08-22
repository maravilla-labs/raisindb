"""
Send Email Notification (stewardship).

Delivers one stewardship email - a ward invitation, a stewardship request -
through the tenant's outbound email configuration (`/config/email` in
`raisin:system`). No provider is named, so the tenant's DEFAULT sender is used:
the same account magic-link sign-in goes through.

The sender identity is deliberately not an input. It comes from the
configuration, so editing this function cannot make mail go out as an address
the tenant never verified.
"""


def send_email_notification(input):
    """Send one stewardship email.

    Args:
        input: dict with email, subject, body, optional html_body, optional
               template / template_data, optional provider.

    Returns:
        dict with sent, message_id, provider, sender.
    """
    email = input.get("email", "")
    subject = input.get("subject", "")
    body = input.get("body", "")
    html_body = input.get("html_body", "")
    template = input.get("template", "")
    template_data = input.get("template_data", {})
    provider = input.get("provider", "")

    if not email or "@" not in email:
        log.warn("[EMAIL] Invalid email address: " + str(email))
        fail("Invalid email address: " + str(email))
    if not subject:
        fail("subject is required")

    # `template` names wording this function does not carry. It is recorded
    # rather than silently ignored, because a caller that expected a template
    # to be rendered would otherwise see a plausible-looking send of the raw
    # body and never learn the template did nothing.
    if template:
        log.info("[EMAIL] template '" + str(template) + "' requested; sending the supplied body")
        if template_data:
            log.info("[EMAIL] template data keys: " + str(sorted(template_data.keys())))

    if not body:
        if not html_body:
            fail("body or html_body is required")
        # An HTML-only message is a spam signal and unreadable in a text
        # client, so a text part is always sent.
        body = "This message requires an HTML-capable mail client."

    message = {
        "to": email,
        "subject": subject,
        "text": body,
    }
    if html_body:
        message["html"] = html_body
    if provider:
        message["provider"] = provider

    receipt = raisin.email.send(message)

    log.info("[EMAIL] sent to " + str(email) + " via " + str(receipt.get("sender", "default")))

    return {
        "sent": True,
        "message_id": receipt.get("message_id", ""),
        "provider": receipt.get("provider", ""),
        "sender": receipt.get("sender", ""),
    }
