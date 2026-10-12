{pkgs}:
pkgs.runCommand "execution-protocol-conformance" {
  nativeBuildInputs = [pkgs.nodejs pkgs.python3];
} ''
  mkdir -p runtime src scripts tests
  cp -r ${../runtime/execution} runtime/execution
  cp -r ${../src/execution_protocol} src/execution_protocol
  cp ${../scripts/generate-execution-protocol.py} scripts/generate-execution-protocol.py
  cp ${../tests/execution-protocol-fixtures.json} tests/execution-protocol-fixtures.json
  python3 scripts/generate-execution-protocol.py --check
  node --test runtime/execution/protocol.test.ts
  touch "$out"
''
