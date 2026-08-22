"""
Send Email Notification.

Delivers one notification through the tenant's outbound email configuration
(`/config/email` in `raisin:system`). No provider is named, so the tenant's
DEFAULT sender is used - the same account magic-link sign-in goes through.
A caller that needs another account passes `provider`.

The sender identity (from address, display name, reply-to) is deliberately not
an input: it comes from the configuration, so a change to this function cannot
make mail go out as an address the tenant never verified.
"""

def _first(input, keys, fallback=""):
    """First non-empty value among `keys`.

    The callers of this handler grew up separately and spell the recipient
    three ways (`to_email`, `recipient_email`, `email`). Accepting all three
    is what keeps a rename in one caller from silently addressing mail to "".
    """
    for key in keys:
        value = input.get(key, None)
        if value:
            return value
    return fallback


def send_email_notification(input):
    """Send one notification email.

    Args:
        input: dict with to_email / recipient_email / email, subject, body
               (or `text`), optional html_body / html, optional provider.

    Returns:
        dict with sent, message_id, provider - or sent=False and a reason.
    """
    to_email = _first(input, ["to_email", "recipient_email", "email", "to"])
    subject = _first(input, ["subject"])
    text = _first(input, ["body", "text", "message"])
    html = _first(input, ["html_body", "html"])
    provider = _first(input, ["provider"])

    if not to_email or "@" not in to_email:
        fail("recipient is not an email address: " + str(to_email))
    if not subject:
        fail("subject is required")
    if not text:
        # An HTML-only message is a spam signal and unreadable in a text
        # client, so a text part is derived rather than sent without one.
        if not html:
            fail("body is required")
        text = "This message requires an HTML-capable mail client."

    message = {
        "to": to_email,
        "subject": subject,
        "text": text,
    }
    if html:
        message["html"] = html
    # Absent `provider` means the tenant's default. Passing an empty string
    # would mean the same thing, but leaving the key out keeps the intent
    # readable in a log of the call.
    if provider:
        message["provider"] = provider

    receipt = raisin.email.send(message)

    log.info("[EMAIL] sent to " + str(to_email) + " via " + str(receipt.get("sender", "default")))

    return {
        "sent": True,
        "success": True,
        "message_id": receipt.get("message_id", ""),
        "provider": receipt.get("provider", ""),
        "sender": receipt.get("sender", ""),
    }
