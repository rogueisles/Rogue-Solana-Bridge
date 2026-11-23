// SPDX-License-Identifier: MIT
pragma solidity 0.8.23;

import "./NativeRgnEscrow.sol";

contract QuorumRgnReserve {
    uint8 public constant THRESHOLD = 3;
    uint16 public constant MAX_WINDOW_OPERATIONS = 128;
    uint64 public constant WINDOW_SECONDS = 3600;
    uint64 public constant MAX_APPROVAL_TTL = 300;
    uint64 public constant ADMIN_DELAY = 24 hours;
    bytes32 public constant FUNDING_DOMAIN = 0xd7cda2ba8dcbaf5393c4189c172753eda491b288f5d85a6e7b6096bd2c74009f;
    bytes32 public constant ROTATION_DOMAIN = 0xc29bb62a532088c4d4f795471abacdf1c25bdd24fe65befe2900f038eda81bac;
    bytes32 public constant LIMIT_DOMAIN = 0x4e743a7d38e16f72a0b78f3d7d29a623e507c8d6dc4175e55394fcda1b4a2a26;
    bytes32 public constant RESUME_DOMAIN = 0x840ee55d6b72280612594e5e49df1e31246ad8de6df23713e62471d9a7602d2f;
    uint256 private constant CURVE_ORDER = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141;
    uint256 private constant HALF_CURVE_ORDER = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;

    NativeRgnEscrow public immutable escrow;
    bytes32 public immutable routeId;
    address public immutable coldAdministrator;
    address[4] public signers;
    mapping(address => bool) public isSigner;
    mapping(bytes32 => bool) public usedOrder;
    mapping(bytes32 => bool) public usedSourceProofHash;
    uint32 public epoch = 1;
    uint64 public hourlyLimitUnits = 10_000_000 * 1e6;
    bool public paused;
    bool private entered;

    struct Funding {
        bytes32 orderId;
        bytes32 hashlock;
        address payable recipient;
        uint64 refundAt;
        uint64 splUnits;
        bytes32 remoteTermsHash;
        bytes32 sourceProofHash;
        uint64 approvalExpiry;
    }
    struct Spend { uint64 at; uint64 units; }
    Spend[128] private spends;
    uint16 private start;
    uint16 private count;
    uint64 private grossUnits;
    address[4] public pendingSigners;
    uint64 public governanceNonce;
    uint64 public pendingValidAfter;
    uint64 public pendingLimitUnits;
    uint32 public pendingEpoch;
    uint8 public pendingKind;

    event Funded(bytes32 indexed orderId, bytes32 indexed swapId, bytes32 indexed sourceProofHash,
        uint32 epoch, uint64 units, uint64 rollingGrossUnits);
    event FundingPaused();
    event FundingUnpaused();
    event GovernanceScheduled(uint8 indexed kind, bytes32 indexed digest, uint32 indexed epoch,
        uint64 nonce, uint64 validAfter);
    event GovernanceCancelled(uint8 indexed kind);
    event SignersRotated(uint32 indexed epoch, address[4] signers);
    event HourlyLimitChanged(uint64 previousLimitUnits, uint64 newLimitUnits);

    modifier onlyColdAdministrator() {
        require(msg.sender == coldAdministrator, "cold administrator only");
        _;
    }
    modifier nonReentrant() {
        require(!entered, "reentrancy");
        entered = true;
        _;
        entered = false;
    }

    constructor(NativeRgnEscrow escrow_, bytes32 routeId_, address[4] memory signers_, address coldAdministrator_) {
        require(address(escrow_) != address(0) && address(escrow_).code.length > 0, "invalid escrow");
        require(routeId_ != bytes32(0), "zero route");
        require(coldAdministrator_ != address(0), "zero administrator");
        escrow = escrow_;
        routeId = routeId_;
        coldAdministrator = coldAdministrator_;
        _validateSigners(signers_);
        for (uint256 i = 0; i < 4; i++) {
            signers[i] = signers_[i];
            isSigner[signers_[i]] = true;
        }
    }

    receive() external payable {}

    function fundingDigest(bytes32 orderId, bytes32 hashlock, address payable recipient, uint64 refundAt,
        uint64 splUnits, bytes32 remoteTermsHash, bytes32 sourceProofHash, uint64 approvalExpiry)
        public view returns (bytes32) {
        return _fundingDigest(Funding(orderId, hashlock, recipient, refundAt, splUnits,
            remoteTermsHash, sourceProofHash, approvalExpiry));
    }

    function _fundingDigest(Funding memory item) private view returns (bytes32) {
        bytes memory first = abi.encodePacked(FUNDING_DOMAIN, routeId, address(this), epoch,
            item.orderId, item.hashlock);
        bytes memory second = abi.encodePacked(item.recipient, item.refundAt, item.splUnits,
            item.remoteTermsHash, item.sourceProofHash, item.approvalExpiry);
        return sha256(abi.encodePacked(first, second));
    }

    function fund(bytes32 orderId, bytes32 hashlock, address payable recipient, uint64 refundAt,
        uint64 splUnits, bytes32 remoteTermsHash, bytes32 sourceProofHash, uint64 approvalExpiry,
        bytes[] calldata approvals) external nonReentrant returns (bytes32 swapId) {
        Funding memory item = Funding(orderId, hashlock, recipient, refundAt, splUnits,
            remoteTermsHash, sourceProofHash, approvalExpiry);
        return _fund(item, approvals);
    }

    function _fund(Funding memory item, bytes[] calldata approvals) private returns (bytes32 swapId) {
        require(!paused, "funding paused");
        require(item.orderId != bytes32(0) && item.hashlock != bytes32(0), "zero identifier");
        require(item.remoteTermsHash != bytes32(0) && item.sourceProofHash != bytes32(0), "zero commitment");
        require(item.recipient != address(0) && item.recipient != address(this), "invalid recipient");
        require(item.splUnits > 0 && item.splUnits <= hourlyLimitUnits, "per swap cap");
        require(item.approvalExpiry > block.timestamp && item.approvalExpiry <= item.refundAt, "approval expired");
        require(item.approvalExpiry <= block.timestamp + MAX_APPROVAL_TTL, "approval lifetime");
        require(!usedOrder[item.orderId], "order used");
        require(!usedSourceProofHash[item.sourceProofHash], "source proof used");
        _verifyApprovals(_fundingDigest(item), approvals);
        _prune();
        require(uint256(grossUnits) + item.splUnits <= hourlyLimitUnits, "rolling cap");
        _recordSpend(item.splUnits);
        usedOrder[item.orderId] = true;
        usedSourceProofHash[item.sourceProofHash] = true;
        swapId = escrow.lock{value: uint256(item.splUnits) * 1e12}(item.orderId, item.hashlock,
            item.recipient, item.refundAt, item.splUnits, item.remoteTermsHash);
        emit Funded(item.orderId, swapId, item.sourceProofHash, epoch, item.splUnits, grossUnits);
    }

    function _verifyApprovals(bytes32 digest, bytes[] calldata approvals) private view {
        require(approvals.length >= THRESHOLD && approvals.length <= 4, "approval count");
        address[4] memory recovered;
        for (uint256 i = 0; i < approvals.length; i++) {
            bytes calldata signature = approvals[i];
            require(signature.length == 65, "signature length");
            bytes32 r;
            bytes32 s;
            uint8 v;
            assembly {
                r := calldataload(signature.offset)
                s := calldataload(add(signature.offset, 32))
                v := byte(0, calldataload(add(signature.offset, 64)))
            }
            require(v == 27 || v == 28, "signature v");
            require(uint256(r) > 0 && uint256(r) < CURVE_ORDER, "signature r");
            require(uint256(s) > 0 && uint256(s) <= HALF_CURVE_ORDER, "signature s");
            address signer = ecrecover(digest, v, r, s);
            require(signer != address(0) && isSigner[signer], "unauthorized signer");
            for (uint256 j = 0; j < i; j++) require(recovered[j] != signer, "duplicate signer");
            recovered[i] = signer;
        }
    }

    function _recordSpend(uint64 units) private {
        uint64 now_ = uint64(block.timestamp);
        if (count > 0 && spends[(start + count - 1) % MAX_WINDOW_OPERATIONS].at == now_) {
            spends[(start + count - 1) % MAX_WINDOW_OPERATIONS].units += units;
        } else {
            require(count < MAX_WINDOW_OPERATIONS, "window operation cap");
            spends[(start + count) % MAX_WINDOW_OPERATIONS] = Spend(now_, units);
            count++;
        }
        grossUnits += units;
    }

    function _prune() private {
        uint16 cursor = start;
        uint16 remaining = count;
        uint64 retainedGross = grossUnits;
        while (remaining > 0 && uint256(spends[cursor].at) + WINDOW_SECONDS <= block.timestamp) {
            retainedGross -= spends[cursor].units;
            cursor = (cursor + 1) % MAX_WINDOW_OPERATIONS;
            remaining--;
        }
        start = cursor;
        count = remaining;
        grossUnits = retainedGross;
    }

    function currentGrossUnits() external view returns (uint64 value) {
        for (uint16 i = 0; i < count; i++) {
            Spend memory item = spends[(start + i) % MAX_WINDOW_OPERATIONS];
            if (uint256(item.at) + WINDOW_SECONDS > block.timestamp) value += item.units;
        }
    }

    function _validateSigners(address[4] memory values) private view {
        for (uint256 i = 0; i < 4; i++) {
            require(values[i] != address(0) && values[i] != coldAdministrator && values[i] != address(this), "invalid signer");
            for (uint256 j = 0; j < i; j++) require(values[i] != values[j], "duplicate signer");
        }
    }

    function rotationDigest(address[4] memory nextSigners, uint64 approvalExpiry) public view returns (bytes32) {
        return sha256(abi.encodePacked(ROTATION_DOMAIN, routeId, address(this), epoch, governanceNonce,
            nextSigners[0], nextSigners[1], nextSigners[2], nextSigners[3], approvalExpiry));
    }

    function limitDigest(uint64 newLimitUnits, uint64 approvalExpiry) public view returns (bytes32) {
        return sha256(abi.encodePacked(LIMIT_DOMAIN, routeId, address(this), epoch, governanceNonce,
            newLimitUnits, approvalExpiry));
    }

    function unpauseDigest(uint64 approvalExpiry) public view returns (bytes32) {
        return sha256(abi.encodePacked(RESUME_DOMAIN, routeId, address(this), epoch, governanceNonce,
            approvalExpiry));
    }

    function _schedule(uint8 kind, bytes32 digest, uint64 approvalExpiry, bytes[] calldata approvals) private {
        require(pendingKind == 0, "governance already pending");
        require(approvalExpiry > block.timestamp, "approval expired");
        require(approvalExpiry <= block.timestamp + MAX_APPROVAL_TTL, "approval lifetime");
        _verifyApprovals(digest, approvals);
        pendingKind = kind;
        pendingEpoch = epoch;
        pendingValidAfter = uint64(block.timestamp + ADMIN_DELAY);
        uint64 nonce = governanceNonce;
        governanceNonce++;
        emit GovernanceScheduled(kind, digest, epoch, nonce, pendingValidAfter);
    }

    function _clearGovernance() private {
        delete pendingSigners;
        delete pendingLimitUnits;
        delete pendingValidAfter;
        delete pendingEpoch;
        delete pendingKind;
    }

    function _execute(uint8 kind) private {
        require(pendingKind == kind && block.timestamp >= pendingValidAfter, "governance delayed");
        require(pendingEpoch == epoch, "governance epoch changed");
        _clearGovernance();
    }

    function scheduleSignerRotation(address[4] calldata nextSigners, uint64 approvalExpiry,
        bytes[] calldata approvals) external onlyColdAdministrator {
        _validateSigners(nextSigners);
        bytes32 digest = rotationDigest(nextSigners, approvalExpiry);
        _schedule(1, digest, approvalExpiry, approvals);
        pendingSigners = nextSigners;
    }

    function executeSignerRotation() external {
        address[4] memory values = pendingSigners;
        _execute(1);
        for (uint256 i = 0; i < 4; i++) isSigner[signers[i]] = false;
        for (uint256 i = 0; i < 4; i++) {
            signers[i] = values[i];
            isSigner[values[i]] = true;
        }
        epoch++;
        emit SignersRotated(epoch, values);
    }

    function scheduleLimitChange(uint64 newLimitUnits, uint64 approvalExpiry,
        bytes[] calldata approvals) external onlyColdAdministrator {
        require(newLimitUnits > 0 && newLimitUnits != hourlyLimitUnits, "invalid limit");
        _schedule(2, limitDigest(newLimitUnits, approvalExpiry), approvalExpiry, approvals);
        pendingLimitUnits = newLimitUnits;
    }

    function executeLimitChange() external {
        uint64 previous = hourlyLimitUnits;
        uint64 next = pendingLimitUnits;
        _execute(2);
        hourlyLimitUnits = next;
        emit HourlyLimitChanged(previous, next);
    }

    function cancelGovernance() external onlyColdAdministrator {
        require(pendingKind != 0, "no pending governance");
        uint8 kind = pendingKind;
        _clearGovernance();
        emit GovernanceCancelled(kind);
    }

    function pauseFunding() external onlyColdAdministrator {
        paused = true;
        if (pendingKind != 0) {
            uint8 kind = pendingKind;
            _clearGovernance();
            emit GovernanceCancelled(kind);
        }
        emit FundingPaused();
    }

    function scheduleUnpause(uint64 approvalExpiry, bytes[] calldata approvals) external onlyColdAdministrator {
        require(paused, "not paused");
        _schedule(3, unpauseDigest(approvalExpiry), approvalExpiry, approvals);
    }

    function executeUnpause() external {
        require(paused, "not paused");
        _execute(3);
        paused = false;
        emit FundingUnpaused();
    }
}
