# Managing contacts

Contacts allow you to save payment identifiers for quick access. Each contact stores a name and payment identifier (such as a Lightning address, BOLT12 offer, or BIP353 address), making it easy to send payments to frequently used recipients without re-entering their identifier. Contacts are synced across all instances of the SDK.

<h2 id="adding-a-contact">
    <a class="header" href="#adding-a-contact">Adding a contact</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_contact">API docs</a>
</h2>

To add a new contact, provide a name and payment identifier.

{{#tabs contacts:add-contact}}

<h2 id="updating-a-contact">
    <a class="header" href="#updating-a-contact">Updating a contact</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.update_contact">API docs</a>
</h2>

To update an existing contact, provide the contact ID along with the new name and payment identifier.

{{#tabs contacts:update-contact}}

<h2 id="deleting-a-contact">
    <a class="header" href="#deleting-a-contact">Deleting a contact</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.delete_contact">API docs</a>
</h2>

To remove a contact, pass its ID to the delete method.

{{#tabs contacts:delete-contact}}

<h2 id="listing-contacts">
    <a class="header" href="#listing-contacts">Listing contacts</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_contacts">API docs</a>
</h2>

To retrieve your saved contacts, use the list method. The results support pagination through offset and limit parameters. You can also filter by exact name match using the optional name parameter.

{{#tabs contacts:list-contacts}}
