# Reserving leaves

By default, the leaves selected during {{#name prepare_send_payment}} are not locked, meaning they could be consumed by another concurrent payment before {{#name send_payment}} is called. Setting {{#name reserve_leaves}} to `true` in the prepare request locks the selected leaves until the payment is sent or explicitly cancelled.

This may be helpful when handling multiple concurrent payments where the wallet's balance may change between prepare and send — for example, in automated or batch payment systems. Reserving ensures the prepared payment won't fail due to concurrent balance changes. For Bitcoin address payments, it also prevents fee quote mismatches caused by different leaves being selected at send time.

## Preparing with reserved leaves

Set {{#name reserve_leaves}} to `true` in the {{#name prepare_send_payment}} request. The prepare response will include a {{#name reservation_id}}.

{{#tabs reserve_leaves:prepare-send-payment-reserve-leaves}}

## Cancelling a reservation

If the user decides not to proceed with the payment, call {{#name cancel_prepare_send_payment}} with the {{#name reservation_id}} to release the reserved leaves.

{{#tabs reserve_leaves:cancel-prepare-send-payment}}

## Notes

- The balance returned by {{#name get_info}} decreases when leaves are reserved, and increases again if the reservation is cancelled or expires.
- Reservations automatically expire after 5 minutes. Calling {{#name send_payment}} after expiry will fail.
- {{#name reserve_leaves}} is ignored for token payments.
- When {{#name reserve_leaves}} is not set or is `false`, no reservation is created during prepare. Leaves are only locked at send time.
