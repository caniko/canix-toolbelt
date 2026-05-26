# Reusable DNS record fragments.
{
  # List-returning helpers are meant to be spliced with `++`; single-record
  # helpers are meant to be inserted inline in a records list.

  # Generate proxied/non-proxied apex CNAMEs for a list of subdomain names.
  # Used to compress the "name -> zone-apex" boilerplate in zone records.
  apexCname = {
    zone,
    names,
    proxied ? false,
    ttlAuto ? true,
  }:
    map (n: {
      name = n;
      type = "CNAME";
      data = zone;
      inherit proxied ttlAuto;
    })
    names;

  dmarc = {
    rua,
    policy ? "none",
    subdomainPolicy ? null,
    comment ? null,
  }: let
    ruaAddresses =
      if builtins.isList rua
      then rua
      else [rua];
    ruaValue = builtins.concatStringsSep "," (map (address: "mailto:${address}") ruaAddresses);
  in {
    name = "_dmarc";
    type = "TXT";
    data =
      "v=DMARC1; p=${policy}"
      + (
        if subdomainPolicy != null
        then "; sp=${subdomainPolicy}"
        else ""
      )
      + "; rua=${ruaValue}";
    ttlAuto = true;
    inherit comment;
  };

  tlsRpt = {
    rua,
    comment ? null,
  }: {
    name = "_smtp._tls";
    type = "TXT";
    data = "v=TLSRPTv1; rua=mailto:${rua}";
    ttlAuto = true;
    inherit comment;
  };

  # `id` is the policy-version identifier; bump when the policy file under
  # mta-sts.<zone>/.well-known/mta-sts.txt changes.
  mtaSts = {
    id,
    comment ? null,
  }: {
    name = "_mta-sts";
    type = "TXT";
    data = "v=STSv1; id=${id}";
    ttlAuto = true;
    inherit comment;
  };

  # Brevo (sendinblue) DKIM CNAMEs. `dkimZone` is the brevo-side zone-style
  # identifier they hand out, for example "tartanoglu-com".
  brevoDkim = {dkimZone}: [
    {
      name = "brevo1._domainkey";
      type = "CNAME";
      data = "b1.${dkimZone}.dkim.brevo.com";
      ttlAuto = true;
    }
    {
      name = "brevo2._domainkey";
      type = "CNAME";
      data = "b2.${dkimZone}.dkim.brevo.com";
      ttlAuto = true;
    }
  ];

  # Cloudflare Pages publish-target alias, always proxied.
  cloudflarePagesCname = {
    name ? "@",
    pagesHost,
  }: {
    inherit name;
    type = "CNAME";
    data = pagesHost;
    proxied = true;
    ttlAuto = true;
  };

  # Namecheap email forwarding (eforward1-5) MX + SPF.
  # DMARC is per-zone (rua addresses differ) and not included here.
  namecheapEmailForwarding = [
    {
      name = "@";
      type = "MX";
      preference = 10;
      data = "eforward1.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "MX";
      preference = 10;
      data = "eforward2.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "MX";
      preference = 10;
      data = "eforward3.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "MX";
      preference = 15;
      data = "eforward4.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "MX";
      preference = 20;
      data = "eforward5.registrar-servers.com";
      ttlAuto = true;
    }
    {
      name = "@";
      type = "TXT";
      data = "v=spf1 include:spf.efwd.registrar-servers.com ~all";
      ttlAuto = true;
    }
  ];
}
