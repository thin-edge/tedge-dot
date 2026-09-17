*** Settings ***
Documentation       Secured OPC UA sessions end to end (doc/connectors/opcua-connector-spec.md):
...                 the connector against python-asyncua servers presenting the certificates of
...                 the shared PKI vectors (genpki.py), with username and X.509 users. One
...                 device per situation (connectors/opcua/secure/connector.toml); the trust
...                 lists are managed with `tedge-dot pki` inside the connector container, as an
...                 operator would.
...
...                 Run via:  just test-e2e opcua   (and just test-e2e-c opcua)

Resource            ../../_shared/stack.resource
Library             Collections

Suite Setup         Setup Secured Stack
Suite Teardown      Teardown OT Stack


*** Variables ***
${PROTOCOL}             opcua
${CONFIG}               /opt/secure/connector.toml
${READY_TIMEOUT}        120
${SAMPLE_TIMEOUT}       20
# A failing device retries on a 1s -> 60s backoff.
${RECONNECT_TIMEOUT}    120
${PASSWORD}             operator-secret-e2e
${WRONG_PASSWORD}       wrong-guess-e2e


*** Test Cases ***
Pinned Server With A Password File Is Connected
    [Documentation]    A pinned server certificate, a username whose password comes from a file.
    ${link}=    Wait For Link Status    pinned    connected
    Should Be Equal    ${{ json.loads($link)["info"]["security_policy"] }}    Basic256Sha256
    Should Be Equal    ${{ json.loads($link)["info"]["security_mode"] }}    sign_and_encrypt
    Should Be Equal    ${{ json.loads($link)["info"]["server_certificate"] }}    trusted
    ${expected}=    Expected Thumbprint    pinned
    Should Be Equal    ${{ json.loads($link)["info"]["server_thumbprint"] }}    ${expected}
    Device Publishes Temperature    pinned

Server Trusted Through Its CA Is Connected
    [Documentation]    Only the root CA (with its CRL) is trusted, not the server certificate.
    Wait For Link Status    ca    connected
    Device Publishes Temperature    ca

X509 User Identity Is Connected
    [Documentation]    An X.509 user identity over Aes256_Sha256_RsaPss.
    ${link}=    Wait For Link Status    x509    connected
    Should Be Equal    ${{ json.loads($link)["info"]["security_policy"] }}    Aes256_Sha256_RsaPss
    Device Publishes Temperature    x509

Untrusted Server Is Quarantined Until Trusted
    [Documentation]    First contact: the certificate lands in rejected/ and the link says why;
    ...                `tedge-dot pki trust` fixes it without a restart.
    ${link}=    Wait For Link Status    untrusted    disconnected
    ${reason}=    Get Json Field    ${link}    reason
    Should Start With    ${reason}    certificate untrusted:
    ${thumbprint}=    Expected Thumbprint    untrusted
    Should Contain    ${reason}    ${thumbprint}
    Should Contain    ${reason}    CN=sim unknown
    ${rejected}=    Pki    list rejected --json
    Should Contain    ${rejected}    ${thumbprint}

    ${mark}=    Get Message Mark
    ${prefix}=    Set Variable    ${thumbprint}[:8]
    Pki    trust ${prefix}
    ${link}=    Wait For Fresh Message With Field    te/device/untrusted/ot/${PROTOCOL}/status/link
    ...    status    connected    timeout=${RECONNECT_TIMEOUT}    since=${mark}
    Should Be Equal    ${{ json.loads($link)["info"]["server_certificate"] }}    trusted
    Device Publishes Temperature    untrusted
    ${rejected}=    Pki    list rejected --json
    Should Not Contain    ${rejected}    ${thumbprint}

Revoked Server Certificate Is Refused
    Link Is Refused    revoked    certificate revoked:

Server Certificate For Another Host Is Refused
    Link Is Refused    wronghost    certificate invalid:

Expired Server Certificate Is Refused
    Link Is Refused    expired    certificate invalid:

Missing CRL Is Reported And Can Be Added
    [Documentation]    A server issued by an intermediate CA whose CRL is missing: revocation
    ...                unknown. Adding the CRL with `tedge-dot pki add-crl` fixes it.
    ${reason}=    Link Is Refused    nocrl    certificate revoked:
    Should Contain    ${reason}    revocation unknown
    ${mark}=    Get Message Mark
    Pki    add-crl /tmp/intermediate.crl
    Wait For Fresh Message With Field    te/device/nocrl/ot/${PROTOCOL}/status/link
    ...    status    connected    timeout=${RECONNECT_TIMEOUT}    since=${mark}
    Device Publishes Temperature    nocrl

Trust Any Server Certificate Accepts An Unknown Server
    ${link}=    Wait For Link Status    trustany    connected
    Should Be Equal    ${{ json.loads($link)["info"]["server_certificate"] }}    not_verified
    Device Publishes Temperature    trustany
    ${rejected}=    Pki    list rejected --json
    ${thumbprint}=    Expected Thumbprint    foreign_ca
    Should Not Contain    ${rejected}    ${thumbprint}

Missing Security Policy Lists What The Server Offers
    ${reason}=    Link Is Refused    noendpoint    no matching endpoint:
    Should Contain    ${reason}    Basic256Sha256/sign_and_encrypt
    Should Contain    ${reason}    Aes256_Sha256_RsaPss/sign_and_encrypt

Wrong Password Is Reported As A Rejected Identity
    ${reason}=    Link Is Refused    badpassword    identity rejected:
    Should Not Contain    ${reason}    ${WRONG_PASSWORD}

Plaintext Password Is Refused Unless Allowed
    Link Is Refused    plaintext    plaintext password refused:
    ${link}=    Wait For Link Status    plaintextok    connected
    Should Be Equal    ${{ json.loads($link)["info"]["security_policy"] }}    None
    Device Publishes Temperature    plaintextok

Application Certificate Was Generated
    [Documentation]    No certificate was configured: the connector generated one in the
    ...                default PKI directory, for its application URI.
    ${out}=    Pki    show --json
    Should Be Equal    ${{ json.loads($out)["application_uri"] }}    urn:tedge-dot
    Should Be True    ${{ json.loads($out)["uri_matches"] }}
    Should Be Equal    ${{ json.loads($out)["certificate"] }}
    ...    /var/lib/tedge-dot/opcua/pki/own/certs/cert.der
    ${mode}=    DeviceLibrary.Execute Command
    ...    cmd=stat -c %a /var/lib/tedge-dot/opcua/pki/own/private/key.pem    strip=${True}
    Should Be Equal    ${mode}    600

Management Command Cannot Point At Local Files
    [Documentation]    A define-device naming a password file is refused before anything is
    ...                written (contract §3.4): the reason names the key, not the path.
    ${device}=    Evaluate
    ...    json.dumps({"name": "sneaky", "protocol_address": {"endpoint": "opc.tcp://simulator-secure:4841/", "user": "operator", "password_file": "/etc/shadow"}, "point": [{"id": "temperature", "datatype": "float64", "address": {"node_id": "ns=2;s=Temperature"}}]})
    ...    modules=json
    ${topic}=    Set Variable    te/device/main/service/tedge-dot/ot/cmd/define-device/local-only-1
    Publish Message    ${topic}    {"status":"init","device":${device}}    retain=True
    ${result}=    Wait For Fresh Message With Field    ${topic}    status    failed
    ...    timeout=${SAMPLE_TIMEOUT}    since=0
    ${reason}=    Get Json Field    ${result}    reason
    Should Contain    ${reason}    device 'sneaky': protocol_address.password_file
    Should Contain    ${reason}    only be set in the configuration file
    Should Not Contain    ${reason}    /etc/shadow
    ${defined}=    DeviceLibrary.Execute Command    cmd=grep -c sneaky ${CONFIG} || true
    ...    strip=${True}
    Should Be Equal    ${defined}    0

Secrets Are Never Published
    [Documentation]    Nothing the connector published -- link statuses, health, samples,
    ...                capabilities -- carries a password.
    Wait For Link Status    badpassword    disconnected
    ${count}=    No Message Contains    ${PASSWORD}    ${WRONG_PASSWORD}
    Should Be True    ${count} > 0


*** Keywords ***
Setup Secured Stack
    Setup OT Stack    opcua    compose_file=${CURDIR}/../docker-compose.secure.yaml
    ${expected}=    DeviceLibrary.Execute Command    cmd=cat /pki-vectors/expected.json
    ...    strip=${True}
    Set Suite Variable    $EXPECTED    ${expected}

Expected Thumbprint
    [Arguments]    ${scenario}
    RETURN    ${{ json.loads($EXPECTED)["scenarios"][$scenario]["thumbprint"] }}

Pki
    [Documentation]    Run `tedge-dot pki <args>` in the connector container; return stdout.
    [Arguments]    ${args}
    ${out}=    DeviceLibrary.Execute Command
    ...    cmd=tedge-dot pki ${args} --config ${CONFIG}    strip=${True}
    RETURN    ${out}

Wait For Link Status
    [Documentation]    Wait for the device's retained link status to be `status`; return it.
    [Arguments]    ${device}    ${status}
    ${topic}=    Set Variable    te/device/${device}/ot/${PROTOCOL}/status/link
    ${payload}=    Wait For Fresh Message With Field    ${topic}    status    ${status}
    ...    timeout=${READY_TIMEOUT}    since=0
    RETURN    ${payload}

Link Is Refused
    [Documentation]    The device is disconnected with a reason starting with `prefix`;
    ...                return the reason.
    [Arguments]    ${device}    ${prefix}
    ${link}=    Wait For Link Status    ${device}    disconnected
    ${reason}=    Get Json Field    ${link}    reason
    Should Start With    ${reason}    ${prefix}
    RETURN    ${reason}

Device Publishes Temperature
    [Arguments]    ${device}
    ${sample}=    Wait For Sample    te/device/${device}/ot/${PROTOCOL}/sample/temperature
    ...    timeout=${SAMPLE_TIMEOUT}
    ${value}=    Get Json Field    ${sample}    value
    Should Be Equal As Numbers    ${value}    21.5
