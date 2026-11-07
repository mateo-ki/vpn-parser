include $(TOPDIR)/rules.mk

PKG_NAME:=luci-app-vproxy
PKG_VERSION:=1.8.8
PKG_RELEASE:=1

PKG_MAINTAINER:=mateo-ki <https://github.com/mateo-ki/vproxy>
PKG_LICENSE:=MIT

LUCI_TITLE:=LuCI Web GUI for VProxy
LUCI_DEPENDS:=+luci-base +vproxy +rpcd +rpcd-mod-luci +curl
LUCI_PKGARCH:=all

include $(TOPDIR)/feeds/luci/luci.mk

# call BuildPackage - OpenWrt buildroot signature
