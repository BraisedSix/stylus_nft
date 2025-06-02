use alloc::{string::String, vec, vec::Vec};
use alloy_primitives::{Address, FixedBytes, U256};
use alloy_sol_types::sol;
use core::{borrow::BorrowMut, marker::PhantomData};
use stylus_sdk::{abi::Bytes, evm, msg, prelude::*};

// 定义 ERC-721 所需的参数 trait
pub trait Erc721Params {
    // NFT 的名称，常量
    const NAME: &'static str;
    // NFT 的符号，常量
    const SYMBOL: &'static str;
    // 获取指定 token_id 的 URI
    fn token_uri(token_id: U256) -> String;
}

// 定义 ERC-721 合约的存储结构
sol_storage! {
    pub struct Erc721<T: Erc721Params> {
        // token_id 到拥有者地址的映射
        mapping(uint256 => address) owners;
        // 地址到余额的映射
        mapping(address => uint256) balances;
        // token_id 到授权用户地址的映射
        mapping(uint256 => address) token_approvals;
        // 拥有者地址到操作者地址的授权映射
        mapping(address => mapping(address => bool)) operator_approvals;
        // 总供应量
        uint256 total_supply;
        // 用于支持 Erc721Params 的 PhantomData
        PhantomData<T> phantom;
    }
}

// 定义事件和 Solidity 错误类型
sol! {
    // 转账事件
    event Transfer(address indexed from, address indexed to, uint256 indexed token_id);
    // 授权事件
    event Approval(address indexed owner, address indexed approved, uint256 indexed token_id);
    // 批量授权事件
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);

    // token_id 未被铸造或已被销毁
    error InvalidTokenId(uint256 token_id);
    // 指定地址不是 token_id 的拥有者
    error NotOwner(address from, uint256 token_id, address real_owner);
    // 指定地址无权操作 token_id
    error NotApproved(address owner, address spender, uint256 token_id);
    // 尝试向零地址转账
    error TransferToZero(uint256 token_id);
    // 接收者拒绝接收 token_id
    error ReceiverRefused(address receiver, uint256 token_id, bytes4 returned);
}

// 定义 ERC-721 错误枚举
#[derive(SolidityError)]
pub enum Erc721Error {
    InvalidTokenId(InvalidTokenId),
    NotOwner(NotOwner),
    NotApproved(NotApproved),
    TransferToZero(TransferToZero),
    ReceiverRefused(ReceiverRefused),
}

// 定义 IERC721TokenReceiver 接口
sol_interface! {
    // 用于调用实现 IERC721TokenReceiver 的合约的 onERC721Received 方法
    interface IERC721TokenReceiver {
        function onERC721Received(address operator, address from, uint256 token_id, bytes data) external returns(bytes4);
    }
}

// 定义 onERC721Received 方法的选择器常量
const ERC721_TOKEN_RECEIVER_ID: u32 = 0x150b7a02;

// 实现 ERC-721 内部方法
impl<T: Erc721Params> Erc721<T> {
    // 检查 msg::sender 是否有权操作指定 token
    fn require_authorized_to_spend(
        &self,
        from: Address,
        token_id: U256,
    ) -> Result<(), Erc721Error> {
        // 获取 token_id 的拥有者
        let owner = self.owner_of(token_id)?;
        // 验证 from 是否为拥有者
        if from != owner {
            return Err(Erc721Error::NotOwner(NotOwner {
                from,
                token_id,
                real_owner: owner,
            }));
        }
        // 如果调用者是拥有者，直接返回
        if msg::sender() == owner {
            return Ok(());
        }
        // 检查调用者是否为拥有者的操作者
        if self.operator_approvals.getter(owner).get(msg::sender()) {
            return Ok(());
        }
        // 检查调用者是否被授权操作此 token
        if msg::sender() == self.token_approvals.get(token_id) {
            return Ok(());
        }
        // 如果无授权，返回错误
        Err(Erc721Error::NotApproved(NotApproved {
            owner,
            spender: msg::sender(),
            token_id,
        }))
    }

    // 执行 token 转账操作
    pub fn transfer(
        &mut self,
        token_id: U256,
        from: Address,
        to: Address,
    ) -> Result<(), Erc721Error> {
        // 获取 token_id 的拥有者
        let mut owner = self.owners.setter(token_id);
        let previous_owner = owner.get();
        // 验证 from 是否为拥有者
        if previous_owner != from {
            return Err(Erc721Error::NotOwner(NotOwner {
                from,
                token_id,
                real_owner: previous_owner,
            }));
        }
        // 更新 token 的拥有者
        owner.set(to);
        // 减少 from 的余额
        let mut from_balance = self.balances.setter(from);
        let balance = from_balance.get() - U256::from(1);
        from_balance.set(balance);
        // 增加 to 的余额
        let mut to_balance = self.balances.setter(to);
        let balance = to_balance.get() + U256::from(1);
        to_balance.set(balance);
        // 清除 token 的授权记录
        self.token_approvals.delete(token_id);
        // 记录转账事件
        evm::log(Transfer { from, to, token_id });
        Ok(())
    }

    // 如果接收者是合约，调用 onERC721Received 方法
    fn call_receiver<S: TopLevelStorage>(
        storage: &mut S,
        token_id: U256,
        from: Address,
        to: Address,
        data: Vec<u8>,
    ) -> Result<(), Erc721Error> {
        // 检查接收者是否为合约
        if to.has_code() {
            // 创建接收者接口实例
            let receiver = IERC721TokenReceiver::new(to);
            // 调用 onERC721Received 方法
            let received = receiver
                .on_erc_721_received(&mut *storage, msg::sender(), from, token_id, data.into())
                .map_err(|_e| {
                    Erc721Error::ReceiverRefused(ReceiverRefused {
                        receiver: receiver.address,
                        token_id,
                        returned: alloy_primitives::FixedBytes(0_u32.to_be_bytes()),
                    })
                })?
                .0;
            // 验证返回的选择器是否正确
            if u32::from_be_bytes(received) != ERC721_TOKEN_RECEIVER_ID {
                return Err(Erc721Error::ReceiverRefused(ReceiverRefused {
                    receiver: receiver.address,
                    token_id,
                    returned: alloy_primitives::FixedBytes(received),
                }));
            }
        }
        Ok(())
    }

    // 执行安全转账并调用 onERC721Received
    pub fn safe_transfer<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        token_id: U256,
        from: Address,
        to: Address,
        data: Vec<u8>,
    ) -> Result<(), Erc721Error> {
        // 执行转账
        storage.borrow_mut().transfer(token_id, from, to)?;
        // 调用接收者检查
        Self::call_receiver(storage, token_id, from, to, data)
    }

    // 铸造新 token 并转账给 to
    pub fn mint(&mut self, to: Address) -> Result<(), Erc721Error> {
        // 获取当前总供应量作为新 token_id
        let new_token_id = self.total_supply.get();
        // 增加总供应量
        self.total_supply.set(new_token_id + U256::from(1u8));
        // 执行转账，从零地址到接收者
        self.transfer(new_token_id, Address::default(), to)?;
        Ok(())
    }

    // 销毁指定 token
    pub fn burn(&mut self, from: Address, token_id: U256) -> Result<(), Erc721Error> {
        // 执行转账到零地址
        self.transfer(token_id, from, Address::default())?;
        Ok(())
    }
}

// 实现 ERC-721 外部方法
#[public]
impl<T: Erc721Params> Erc721<T> {
    // 获取 NFT 名称
    pub fn name() -> Result<String, Erc721Error> {
        Ok(T::NAME.into())
    }

    // 获取 NFT 符号
    pub fn symbol() -> Result<String, Erc721Error> {
        Ok(T::SYMBOL.into())
    }

    // 获取指定 token 的 URI
    #[selector(name = "tokenURI")]
    pub fn token_uri(&self, token_id: U256) -> Result<String, Erc721Error> {
        // 确保 token 存在
        self.owner_of(token_id)?;
        Ok(T::token_uri(token_id))
    }

    // 获取指定地址的 NFT 余额
    pub fn balance_of(&self, owner: Address) -> Result<U256, Erc721Error> {
        Ok(self.balances.get(owner))
    }

    // 获取指定 token 的拥有者
    pub fn owner_of(&self, token_id: U256) -> Result<Address, Erc721Error> {
        // 获取 token 的拥有者
        let owner = self.owners.get(token_id);
        // 如果拥有者是零地址，token 无效
        if owner.is_zero() {
            return Err(Erc721Error::InvalidTokenId(InvalidTokenId { token_id }));
        }
        Ok(owner)
    }

    // 执行带数据的安全转账
    #[selector(name = "safeTransferFrom")]
    pub fn safe_transfer_from_with_data<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        from: Address,
        to: Address,
        token_id: U256,
        data: Bytes,
    ) -> Result<(), Erc721Error> {
        // 禁止转账到零地址
        if to.is_zero() {
            return Err(Erc721Error::TransferToZero(TransferToZero { token_id }));
        }
        // 检查调用者是否有权限
        storage
            .borrow_mut()
            .require_authorized_to_spend(from, token_id)?;
        // 执行安全转账
        Self::safe_transfer(storage, token_id, from, to, data.0)
    }

    // 执行不带数据的安全转账
    #[selector(name = "safeTransferFrom")]
    pub fn safe_transfer_from<S: TopLevelStorage + BorrowMut<Self>>(
        storage: &mut S,
        from: Address,
        to: Address,
        token_id: U256,
    ) -> Result<(), Erc721Error> {
        // 调用带数据的安全转账，数据为空
        Self::safe_transfer_from_with_data(storage, from, to, token_id, Bytes(vec![]))
    }

    // 执行普通转账
    pub fn transfer_from(
        &mut self,
        from: Address,
        to: Address,
        token_id: U256,
    ) -> Result<(), Erc721Error> {
        // 禁止转账到零地址
        if to.is_zero() {
            return Err(Erc721Error::TransferToZero(TransferToZero { token_id }));
        }
        // 检查调用者是否有权限
        self.require_authorized_to_spend(from, token_id)?;
        // 执行转账
        self.transfer(token_id, from, to)?;
        Ok(())
    }

    // 为指定 token 设置授权
    pub fn approve(&mut self, approved: Address, token_id: U256) -> Result<(), Erc721Error> {
        // 获取 token 的拥有者
        let owner = self.owner_of(token_id)?;
        // 验证调用者是否有权限
        if msg::sender() != owner && !self.operator_approvals.getter(owner).get(msg::sender()) {
            return Err(Erc721Error::NotApproved(NotApproved {
                owner,
                spender: msg::sender(),
                token_id,
            }));
        }
        // 设置授权
        self.token_approvals.insert(token_id, approved);
        // 记录授权事件
        evm::log(Approval {
            approved,
            owner,
            token_id,
        });
        Ok(())
    }

    // 设置批量授权
    pub fn set_approval_for_all(
        &mut self,
        operator: Address,
        approved: bool,
    ) -> Result<(), Erc721Error> {
        // 获取调用者地址
        let owner = msg::sender();
        // 设置操作者授权
        self.operator_approvals
            .setter(owner)
            .insert(operator, approved);
        // 记录批量授权事件
        evm::log(ApprovalForAll {
            owner,
            operator,
            approved,
        });
        Ok(())
    }

    // 获取指定 token 的授权地址
    pub fn get_approved(&mut self, token_id: U256) -> Result<Address, Erc721Error> {
        Ok(self.token_approvals.get(token_id))
    }

    // 检查是否为所有者设置了操作者授权
    pub fn is_approved_for_all(
        &mut self,
        owner: Address,
        operator: Address,
    ) -> Result<bool, Erc721Error> {
        Ok(self.operator_approvals.getter(owner).get(operator))
    }

    // 检查是否支持指定接口
    pub fn supports_interface(interface: FixedBytes<4>) -> Result<bool, Erc721Error> {
        // 将接口 ID 转换为字节数组
        let interface_slice_array: [u8; 4] = interface.as_slice().try_into().unwrap();
        // 特殊处理 ERC165 标准中的 0xffffffff
        if u32::from_be_bytes(interface_slice_array) == 0xffffffff {
            return Ok(false);
        }
        // 定义支持的接口 ID
        const IERC165: u32 = 0x01ffc9a7;
        const IERC721: u32 = 0x80ac58cd;
        const IERC721_METADATA: u32 = 0x5b5e139f;
        // 检查是否支持指定接口
        Ok(matches!(
            u32::from_be_bytes(interface_slice_array),
            IERC165 | IERC721 | IERC721_METADATA
        ))
    }
}
