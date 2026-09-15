# Managing contacts

Contacts allow you to save Lightning addresses for quick access. Each contact stores a name and a Lightning address, making it easy to send payments to frequently used recipients. Contacts are synced across all instances of the SDK.

## Adding a contact

To add a new contact, provide a name and a Lightning address.

{{#tabs contacts:add-contact}}

## Updating a contact

To update an existing contact, provide the contact ID along with the new name and Lightning address.

{{#tabs contacts:update-contact}}

## Deleting a contact

To remove a contact, pass its ID to the delete method.

{{#tabs contacts:delete-contact}}

## Listing contacts

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters.

{{#tabs contacts:list-contacts}}
