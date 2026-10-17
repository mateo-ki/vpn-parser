include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-vpn-parser
PKG_VERSION:=1.8.2
PKG_RELEASE:=1

PKG_MAINTAINER:=mateo-ki <https://github.com/mateo-ki/vpn-parser>
PKG_LICENSE:=GPL-3.0-only

LUCI_TITLE:=LuCI Web GUI for VPN Parser
LUCI_DEPENDS:=+luci-base +vpn-parser +rpcd +rpcd-mod-luci +curl
LUCI_PKGARCH:=all

include $(TOPDIR)/feeds/luci/luci.mk

# call BuildPackage - OpenWrt buildroot signature
