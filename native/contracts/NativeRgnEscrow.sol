// SPDX-License-Identifier: MIT
pragma solidity 0.8.23;

/// @notice Funded native RGN escrows. Counterpart verification is performed by the clients.
contract NativeRgnEscrow {
    uint256 public constant WEI_PER_SPL_UNIT = 1e12;
    bytes32 public immutable localDomain;
    bytes32 public immutable remoteDomain;
    bytes32 public immutable remoteAsset;
    bytes32 public immutable remoteProgram;
    uint64 public immutable minLifetime;
    uint64 public immutable maxLifetime;
    uint256 public totalLockedWei;
    uint256 public totalCreditWei;
    mapping(address => uint256) public withdrawableWei;
    bool private entered;

    enum State { Absent, Locked, Claimed, Refunded }
    struct Swap {
        bytes32 orderId;
        bytes32 hashlock;
        bytes32 remoteTermsHash;
        address payable funder;
        address payable recipient;
        uint64 splUnits;
        uint64 lockedAt;
        uint64 refundAt;
        State state;
    }
    mapping(bytes32 => Swap) private swaps;

    event Locked(bytes32 indexed swapId, bytes32 indexed orderId, address indexed funder,
        address recipient, bytes32 hashlock, uint64 splUnits, uint64 refundAt,
        bytes32 remoteTermsHash);
    event Claimed(bytes32 indexed swapId, bytes32 secret, address indexed recipient);
    event Refunded(bytes32 indexed swapId, address indexed funder);
    event Withdrawn(address indexed beneficiary, uint256 value);

    modifier nonReentrant() {
        require(!entered, "reentrancy");
        entered = true;
        _;
        entered = false;
    }

    constructor(bytes32 domain_, bytes32 remoteDomain_, bytes32 asset_, bytes32 program_,
        uint64 minimumLifetime_, uint64 maximumLifetime_) {
        require(domain_ != bytes32(0) && remoteDomain_ != bytes32(0), "zero domain");
        require(domain_ != remoteDomain_, "same domain");
        require(asset_ != bytes32(0) && program_ != bytes32(0), "zero remote identity");
        require(minimumLifetime_ > 0 && maximumLifetime_ >= minimumLifetime_, "lifetime limits");
        require(maximumLifetime_ <= 30 days, "lifetime too long");
        localDomain = domain_;
        remoteDomain = remoteDomain_;
        remoteAsset = asset_;
        remoteProgram = program_;
        minLifetime = minimumLifetime_;
        maxLifetime = maximumLifetime_;
    }

    function computeSwapId(address funder, bytes32 orderId) public view returns (bytes32) {
        return sha256(abi.encodePacked(localDomain, address(this), funder, orderId));
    }

    function lock(bytes32 orderId, bytes32 hashlock, address payable recipient,
        uint64 refundAt, uint64 splUnits, bytes32 remoteTermsHash)
        external payable nonReentrant returns (bytes32 swapId) {
        require(orderId != bytes32(0) && hashlock != bytes32(0), "zero identifier");
        require(remoteTermsHash != bytes32(0), "zero counterpart commitment");
        require(recipient != address(0) && recipient != address(this), "invalid recipient");
        require(splUnits > 0 && msg.value == uint256(splUnits) * WEI_PER_SPL_UNIT, "exact amount");
        require(refundAt >= block.timestamp + minLifetime, "deadline too soon");
        require(refundAt <= block.timestamp + maxLifetime, "deadline too late");
        swapId = computeSwapId(msg.sender, orderId);
        require(swaps[swapId].state == State.Absent, "identifier used");
        swaps[swapId] = Swap(orderId, hashlock, remoteTermsHash, payable(msg.sender), recipient,
            splUnits, uint64(block.timestamp), refundAt, State.Locked);
        totalLockedWei += msg.value;
        emit Locked(swapId, orderId, msg.sender, recipient, hashlock, splUnits, refundAt, remoteTermsHash);
    }

    function claim(bytes32 swapId, bytes32 secret) external nonReentrant {
        Swap storage item = swaps[swapId];
        require(item.state == State.Locked, "not locked");
        require(block.timestamp < item.refundAt, "claim expired");
        require(sha256(abi.encodePacked(secret)) == item.hashlock, "wrong secret");
        uint256 value = uint256(item.splUnits) * WEI_PER_SPL_UNIT;
        item.state = State.Claimed;
        totalLockedWei -= value;
        withdrawableWei[item.recipient] += value;
        totalCreditWei += value;
        emit Claimed(swapId, secret, item.recipient);
    }

    function refund(bytes32 swapId) external nonReentrant {
        Swap storage item = swaps[swapId];
        require(item.state == State.Locked, "not locked");
        require(block.timestamp >= item.refundAt, "refund too soon");
        uint256 value = uint256(item.splUnits) * WEI_PER_SPL_UNIT;
        item.state = State.Refunded;
        totalLockedWei -= value;
        withdrawableWei[item.funder] += value;
        totalCreditWei += value;
        emit Refunded(swapId, item.funder);
    }

    function withdrawFor(address payable beneficiary) external nonReentrant {
        uint256 value = withdrawableWei[beneficiary];
        require(value > 0, "no credit");
        withdrawableWei[beneficiary] = 0;
        totalCreditWei -= value;
        (bool sent,) = beneficiary.call{value: value}("");
        require(sent, "beneficiary rejected");
        emit Withdrawn(beneficiary, value);
    }

    function getSwap(bytes32 swapId) external view returns (Swap memory) {
        return swaps[swapId];
    }
}

/// @notice Staging reserve with an exact, bounded rolling gross funding budget.
/// @dev It never owns user escrow storage and has no power to change escrow recipients.
contract RgnReserveVault {
    uint16 public constant MAX_WINDOW_OPERATIONS = 128;
    uint64 public constant ADMIN_DELAY = 48 hours;
    NativeRgnEscrow public immutable escrow;
    address public immutable administrator;
    uint64 public immutable maxPerSwapUnits;
    uint64 public immutable maxWindowUnits;
    uint64 public immutable windowSeconds;
    address public operator;
    bool public paused;
    bool private entered;
    address public pendingOperator;
    uint64 public operatorValidAfter;
    struct Withdrawal { address payable to; uint64 units; uint64 validAfter; }
    Withdrawal public pendingWithdrawal;
    struct Spend { uint64 at; uint64 units; }
    Spend[128] private spends;
    uint16 private start;
    uint16 private count;
    uint64 private grossUnits;

    event ReserveFunded(bytes32 indexed swapId, uint64 units, uint64 grossUnits);
    event Paused(bool paused);
    event OperatorProposed(address indexed operator, uint64 validAfter);
    event OperatorChanged(address indexed operator);
    event WithdrawalProposed(address indexed to, uint64 units, uint64 validAfter);
    event Withdrawn(address indexed to, uint64 units);

    modifier onlyAdministrator() { require(msg.sender == administrator, "administrator only"); _; }
    modifier nonReentrant() {
        require(!entered, "reentrancy"); entered = true; _; entered = false;
    }

    constructor(NativeRgnEscrow escrow_, address administrator_, address operator_,
        uint64 maxPerSwapUnits_, uint64 maxWindowUnits_, uint64 windowSeconds_) {
        require(address(escrow_) != address(0) && address(escrow_).code.length > 0, "invalid escrow");
        require(administrator_ != address(0) && operator_ != address(0), "zero authority");
        require(administrator_ != operator_, "separate authorities");
        require(maxPerSwapUnits_ > 0 && maxWindowUnits_ >= maxPerSwapUnits_, "invalid caps");
        require(windowSeconds_ >= 60 && windowSeconds_ <= 7 days, "invalid window");
        escrow = escrow_;
        administrator = administrator_;
        operator = operator_;
        maxPerSwapUnits = maxPerSwapUnits_;
        maxWindowUnits = maxWindowUnits_;
        windowSeconds = windowSeconds_;
    }

    receive() external payable {}

    function setPaused(bool value) external onlyAdministrator {
        paused = value;
        emit Paused(value);
    }

    function fund(bytes32 orderId, bytes32 hashlock, address payable recipient,
        uint64 refundAt, uint64 splUnits, bytes32 remoteTermsHash)
        external nonReentrant returns (bytes32 swapId) {
        require(msg.sender == operator, "operator only");
        require(!paused, "funding paused");
        require(splUnits > 0 && splUnits <= maxPerSwapUnits, "per swap cap");
        _prune();
        require(uint256(grossUnits) + splUnits <= maxWindowUnits, "rolling cap");
        uint64 now_ = uint64(block.timestamp);
        if (count > 0 && spends[(start + count - 1) % MAX_WINDOW_OPERATIONS].at == now_) {
            spends[(start + count - 1) % MAX_WINDOW_OPERATIONS].units += splUnits;
        } else {
            require(count < MAX_WINDOW_OPERATIONS, "window operation cap");
            spends[(start + count) % MAX_WINDOW_OPERATIONS] = Spend(now_, splUnits);
            count++;
        }
        grossUnits += splUnits;
        swapId = escrow.lock{value: uint256(splUnits) * 1e12}(
            orderId, hashlock, recipient, refundAt, splUnits, remoteTermsHash);
        emit ReserveFunded(swapId, splUnits, grossUnits);
    }

    function currentGrossUnits() external view returns (uint64 value) {
        for (uint16 i = 0; i < count; i++) {
            Spend memory item = spends[(start + i) % MAX_WINDOW_OPERATIONS];
            if (uint256(item.at) + windowSeconds > block.timestamp) value += item.units;
        }
    }

    function _prune() private {
        uint16 cursor = start;
        uint16 remaining = count;
        uint64 retainedGross = grossUnits;
        while (remaining > 0 && uint256(spends[cursor].at) + windowSeconds <= block.timestamp) {
            retainedGross -= spends[cursor].units;
            cursor = (cursor + 1) % MAX_WINDOW_OPERATIONS;
            remaining--;
        }
        start = cursor;
        count = remaining;
        grossUnits = retainedGross;
    }

    function proposeOperator(address value) external onlyAdministrator {
        require(value != address(0) && value != administrator, "invalid operator");
        pendingOperator = value;
        operatorValidAfter = uint64(block.timestamp + ADMIN_DELAY);
        emit OperatorProposed(value, operatorValidAfter);
    }

    function activateOperator() external onlyAdministrator {
        require(pendingOperator != address(0) && block.timestamp >= operatorValidAfter, "operator delayed");
        operator = pendingOperator;
        delete pendingOperator;
        delete operatorValidAfter;
        emit OperatorChanged(operator);
    }

    function proposeWithdrawal(address payable to, uint64 units) external onlyAdministrator {
        require(paused && to != address(0) && units > 0, "invalid withdrawal");
        pendingWithdrawal = Withdrawal(to, units, uint64(block.timestamp + ADMIN_DELAY));
        emit WithdrawalProposed(to, units, pendingWithdrawal.validAfter);
    }

    function executeWithdrawal() external onlyAdministrator nonReentrant {
        Withdrawal memory request = pendingWithdrawal;
        require(paused && request.units > 0 && block.timestamp >= request.validAfter, "withdrawal delayed");
        delete pendingWithdrawal;
        (bool sent,) = request.to.call{value: uint256(request.units) * 1e12}("");
        require(sent, "withdrawal rejected");
        emit Withdrawn(request.to, request.units);
    }
}
