import http.client
import ipaddress
import json
import os
import sys
import time
from pathlib import Path
from urllib.parse import quote, urlencode


class UpdateError(Exception):
    pass


def request(host, path, *, method="GET", headers=None, payload=None):
    connection = http.client.HTTPSConnection(host, timeout=10)
    try:
        headers = dict(headers or {})
        if payload is not None:
            headers["Content-Type"] = "application/json"
        connection.request(
            method, path, None if payload is None else json.dumps(payload), headers
        )
        response = connection.getresponse()
        if not 200 <= response.status < 300:
            raise UpdateError(f"{host} returned HTTP {response.status}")
        body = response.read(4 * 1024 * 1024 + 1)
        if len(body) > 4 * 1024 * 1024:
            raise UpdateError(f"{host} returned an oversized response")
        return body
    finally:
        connection.close()


def public_address(version):
    address = ipaddress.ip_address(request(f"checkipv{version}.dedyn.io", "/").strip().decode("ascii"))
    if address.version != version or not address.is_global or address.is_multicast:
        raise UpdateError(f"IP echo did not return a public IPv{version} address")
    return str(address)


def update(kind, token, zone, fqdn, addresses):
    owner = "" if fqdn == zone else fqdn.removesuffix("." + zone)
    if kind == "desec":
        # https://desec.readthedocs.io/en/latest/dyndns/update-api.html
        query = urlencode({
            "hostname": fqdn,
            "myipv4": addresses.get("A", ""),
            "myipv6": addresses.get("AAAA", ""),
        })
        body = request("update.dedyn.io", "/?" + query, headers={"Authorization": f"Token {token}"})
        if body.strip() != b"good":
            raise UpdateError("deSEC rejected the update")
    elif kind == "hetzner":
        # https://docs.hetzner.cloud/reference/cloud#zone-rrset-actions-set-records-of-an-rrset
        headers = {"Authorization": f"Bearer {token}"}
        host = "api.hetzner.cloud"
        for record_type, address in addresses.items():
            path = f"/v1/zones/{quote(zone, safe='')}/rrsets/{quote(owner or '@', safe='')}/{record_type}"
            rrset = json.loads(request(host, path, headers=headers))["rrset"]
            if [record["value"] for record in rrset["records"]] == [address]:
                continue
            action = json.loads(request(
                host, path + "/actions/set_records", method="POST", headers=headers,
                payload={"records": [{"value": address}]},
            ))["action"]
            for _ in range(10):
                if action["status"] != "running":
                    break
                time.sleep(1)
                action = json.loads(request(
                    host, f"/v1/zones/actions/{int(action['id'])}", headers=headers
                ))["action"]
            if action["status"] != "success":
                raise UpdateError("Hetzner did not complete the RRset update")
    elif kind == "gcore":
        # https://docs.gcore.com/api-reference/dns/rrsets/update-rrset
        headers = {"Authorization": f"apikey {token}"}
        for record_type, address in addresses.items():
            request(
                "api.gcore.com",
                f"/dns/v2/zones/{quote(zone, safe='')}/{quote(fqdn, safe='')}/{record_type}",
                method="PUT", headers=headers,
                payload={"resource_records": [{"content": [address], "enabled": True}]},
            )
    elif kind == "bunny":
        # https://docs.bunny.net/api-reference/core/dns-zone/update-dns-record
        headers = {"AccessKey": token}
        host = "api.bunny.net"
        query = urlencode({"search": zone, "view": 1})
        zones = json.loads(request(host, "/dnszone?" + query, headers=headers))["Items"]
        matches = [item for item in zones if item["Domain"].lower().rstrip(".") == zone]
        if len(matches) != 1:
            raise UpdateError("Bunny DNS zone not found uniquely")
        path = f"/dnszone/{int(matches[0]['Id'])}"
        zone_data = json.loads(request(host, path, headers=headers))
        for record_type, address in addresses.items():
            match_type = 0 if record_type == "A" else 1
            matches = [record for record in zone_data["Records"] if
                       record["Name"].lower() == owner and record["Type"] == match_type]
            if len(matches) != 1:
                raise UpdateError(f"Bunny DNS needs one existing {record_type} record for {fqdn}")
            if matches[0]["Value"] != address:
                request(
                    host, f"{path}/records/{int(matches[0]['Id'])}", method="POST",
                    headers=headers, payload={"Value": address},
                )
    else:
        raise UpdateError("Unsupported DDNS provider")


def main():
    kind, credential, zone, fqdn, *families = sys.argv[1:]
    try:
        token = (Path(os.environ["CREDENTIALS_DIRECTORY"]) / credential).read_text().strip()
        if not token or not all("!" <= char <= "~" for char in token):
            raise UpdateError("Invalid DNS credential file")
        addresses = {}
        for family in families:
            record_type, version = {"ipv4": ("A", 4), "ipv6": ("AAAA", 6)}[family]
            addresses[record_type] = public_address(version)
        update(kind, token, zone, fqdn, addresses)
    except UpdateError as error:
        sys.exit(str(error))
    except (OSError, http.client.HTTPException, ValueError, KeyError, TypeError) as error:
        sys.exit(f"{kind} DDNS update failed ({type(error).__name__})")


if __name__ == "__main__":
    main()
