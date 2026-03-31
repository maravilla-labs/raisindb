"""
Send Email Notification (Placeholder)

Placeholder function for sending email notifications.
Currently logs the email details. Replace with actual email service integration.
"""

def send_email_notification(input):
    """Send an email notification.

    This is a placeholder that logs what would be sent.
    Replace with actual email service (SendGrid, Mailgun, SES, etc.).

    Args:
        input: dict with email, subject, body, html_body, template, template_data

    Returns:
        dict with sent (bool), reason (str), and message_id (str)
    """
    email = input.get("email", "")
    subject = input.get("subject", "")
    body = input.get("body", "")
    html_body = input.get("html_body", "")
    template = input.get("template", "")
    template_data = input.get("template_data", {})

    # Validate email
    if not email or "@" not in email:
        log.warn("[EMAIL] Invalid email address: " + str(email))
        fail("Invalid email address: " + str(email))

    # Log what would be sent
    log.info("[EMAIL] Would send email notification")
    log.info("[EMAIL] To: " + str(email))
    log.info("[EMAIL] Subject: " + str(subject))

    if template:
        log.info("[EMAIL] Using template: " + str(template))
        if template_data:
            log.info("[EMAIL] Template data: " + str(template_data))
    else:
        if body:
            log.info("[EMAIL] Body (text): " + str(body)[:100] + ("..." if len(str(body)) > 100 else ""))
        if html_body:
            log.info("[EMAIL] Body (HTML): " + str(len(html_body)) + " characters")

    # Return placeholder response
    return {
        "sent": False,
        "reason": "email_not_configured",
        "would_send_to": email,
        "subject": subject
    }
