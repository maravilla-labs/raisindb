"""
Send Push Notification (Placeholder)

Placeholder function for sending push notifications.
Currently logs the notification details. Replace with actual push service integration.
"""

def send_push_notification(input):
    """Send a push notification to a user.

    This is a placeholder that logs what would be sent.
    Replace with actual push notification service (Firebase, OneSignal, etc.).

    Args:
        input: dict with user_id, title, body, data, priority

    Returns:
        dict with sent (bool) and reason (str)
    """
    user_id = input.get("user_id", "unknown")
    title = input.get("title", "")
    body = input.get("body", "")
    data = input.get("data", {})
    priority = input.get("priority", "normal")

    # Log what would be sent
    log.info("[PUSH] Would send push notification")
    log.info("[PUSH] To user: " + str(user_id))
    log.info("[PUSH] Title: " + str(title))
    log.info("[PUSH] Body: " + str(body))
    log.info("[PUSH] Priority: " + str(priority))
    if data:
        log.info("[PUSH] Data payload: " + str(data))

    # Return placeholder response
    return {
        "sent": False,
        "reason": "push_not_configured",
        "would_send_to": user_id,
        "title": title,
        "body": body
    }
