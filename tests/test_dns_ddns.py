import json
import runpy
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

update = runpy.run_path(
    str(Path(__file__).resolve().parents[1] / "modules/nixos/services/dns-ddns.py")
)["update"]


class DdnsUpdateTest(unittest.TestCase):
    def test_provider_requests(self):
        def encoded(value):
            return json.dumps(value).encode()

        desec = Mock(return_value=b"good")
        hetzner = Mock(side_effect=[
            encoded({"rrset": {"records": [{"value": "192.0.2.2"}]}}),
            encoded({"action": {"status": "success"}}),
        ])
        gcore = Mock(return_value=b"{}")
        bunny = Mock(side_effect=[
            encoded({"Items": [{"Id": 3, "Domain": "example.org"}]}),
            encoded({"Records": [{"Id": 7, "Name": "sub", "Type": 0, "Value": "192.0.2.2"}]}),
            b"",
        ])
        for kind, request in [("desec", desec), ("hetzner", hetzner), ("gcore", gcore), ("bunny", bunny)]:
            with self.subTest(kind=kind), patch.dict(update.__globals__, request=request):
                fqdn = "example.org" if kind == "hetzner" else "sub.example.org"
                update(kind, "fixture-token", "example.org", fqdn, {"A": "192.0.2.1"})

        desec.assert_called_once_with(
            "update.dedyn.io", "/?hostname=sub.example.org&myipv4=192.0.2.1&myipv6=",
            headers={"Authorization": "Token fixture-token"},
        )
        hetzner.assert_called_with(
            "api.hetzner.cloud", "/v1/zones/example.org/rrsets/@/A/actions/set_records",
            method="POST", headers={"Authorization": "Bearer fixture-token"},
            payload={"records": [{"value": "192.0.2.1"}]},
        )
        gcore.assert_called_once_with(
            "api.gcore.com", "/dns/v2/zones/example.org/sub.example.org/A",
            method="PUT", headers={"Authorization": "apikey fixture-token"},
            payload={"resource_records": [{"content": ["192.0.2.1"], "enabled": True}]},
        )
        bunny.assert_called_with(
            "api.bunny.net", "/dnszone/3/records/7", method="POST",
            headers={"AccessKey": "fixture-token"}, payload={"Value": "192.0.2.1"},
        )


if __name__ == "__main__":
    unittest.main()
