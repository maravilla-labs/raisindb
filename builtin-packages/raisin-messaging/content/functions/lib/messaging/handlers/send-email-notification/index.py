"""
Send Email Notification (Placeholder)

Placeholder function for sending email notifications.
Currently logs the notification details. Replace with actual email service integration.
"""

def send_email_notification(input):
    """Send an email notification.

    This is a placeholder that logs what would be sent.
    Replace with actual email service (SendGrid, SES, etc.).

    Args:
        input: dict with to_email, subject, body, data

    Returns:
        dict with sent (bool) and reason (str)
    """
    to_email = input.get("to_email", "unknown")
    subject = input.get("subject", "")
    body = input.get("body", "")
    data = input.get("data", {})

    # Log what would be sent
    log.info("[EMAIL] Would send email notification")
    log.info("[EMAIL] To: " + str(to_email))
    log.info("[EMAIL] Subject: " + str(subject))
    log.info("[EMAIL] Body: " + str(body))
    if data:
        log.info("[EMAIL] Data payload: " + str(data))

    # Return placeholder response
    return {
        "sent": False,
        "reason": "email_not_configured",
        "would_send_to": to_email,
        "subject": subject,
        "body": body
    }
