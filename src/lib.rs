// 如果未启用 export-abi 特性，仅作为 WASM 运行
#![cfg_attr(not(any(feature = "export-abi", test)), no_main)]
extern crate alloc;

// 引入模块和依赖
mod erc721;

use crate::erc721::{Erc721, Erc721Error, Erc721Params};
use alloy_primitives::{Address, U256};
// 引入 Stylus SDK 和 alloy 基本类型
use stylus_sdk::{msg, prelude::*};

// 定义 NFT 参数结构体
struct StylusNFTParams;
// 实现 Erc721Params trait
impl Erc721Params for StylusNFTParams {
    // 定义 NFT 名称常量
    const NAME: &'static str = "DOG";
    // 定义 NFT 符号常量
    const SYMBOL: &'static str = "DOG";
    // 生成指定 token_id 的 URI
    fn token_uri(token_id: U256) -> String {
        format!("{}{}{}", "https://external-magenta-alpaca.myfilebase.com/ipfs/QmY47C6mUFEGPGF5muGTEcSD3MPspCSpT2EGJV8QvQGUnV", token_id, ".json")
    }
}

// 定义合约入口点和存储结构
sol_storage! {
    #[entrypoint]
    struct StylusNFT {
        // 允许 erc721 访问 StylusNFT 的存储并调用方法
        #[borrow]
        Erc721<StylusNFTParams> erc721;
    }
}

// 实现 StylusNFT 的外部方法
#[public]
#[inherit(Erc721<StylusNFTParams>)]
impl StylusNFT {
    // 铸造 NFT 给调用者
    pub fn mint(&mut self) -> Result<(), Erc721Error> {
        // 获取调用者地址
        let minter = msg::sender();
        // 调用 erc721 的 mint 方法
        self.erc721.mint(minter)?;
        Ok(())
    }

    // 铸造 NFT 给指定地址
    pub fn mint_to(&mut self, to: Address) -> Result<(), Erc721Error> {
        // 调用 erc721 的 mint 方法
        self.erc721.mint(to)?;
        Ok(())
    }

    // 销毁指定 NFT
    pub fn burn(&mut self, token_id: U256) -> Result<(), Erc721Error> {
        // 调用 erc721 的 burn 方法，验证调用者是否拥有 token
        self.erc721.burn(msg::sender(), token_id)?;
        Ok(())
    }

    // 获取总供应量
    pub fn total_supply(&mut self) -> Result<U256, Erc721Error> {
        // 获取 erc721 的总供应量
        Ok(self.erc721.total_supply.get())
    }
}
