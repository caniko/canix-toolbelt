"""octoDNS processor that drops `octodns: { ignored: true }` records from
both source and target zones before planning and provider `_extra_changes`.

Upstream octodns-cloudflare's `_extra_changes` iterates `desired.records`
without checking `record.ignored`, causing RFC 5737 placeholder records
(192.0.2.1 / 2001:db8::1) emitted by dns-manager for type-specific excludes
to overwrite live DDNS records whenever proxied/auto-ttl metadata differs.
"""

from octodns.processor.base import BaseProcessor


class DropIgnoredRecordsProcessor(BaseProcessor):
    def process_source_and_target_zones(
        self, desired, existing, target, lenient=False
    ):
        ignored = {
            (record.name, record._type)
            for record in desired.records
            if getattr(record, "ignored", False)
        }
        if not ignored:
            return desired, existing
        for record in list(desired.records):
            if (record.name, record._type) in ignored:
                desired.remove_record(record)
        for record in list(existing.records):
            if (record.name, record._type) in ignored:
                existing.remove_record(record)
        return desired, existing
