import type { LucideIcon } from "lucide-react"
import {
  Activity, AlertTriangle, Anchor, Archive, ArrowDown, ArrowLeft, ArrowRight, ArrowUp, AtSign,
  Award, BadgeCheck, BadgeInfo, Ban, BarChart3, Bell, Bot, Boxes, Building, Building2, Calendar,
  CalendarClock, CalendarDays, Check, CheckCircle2, ChevronDown, ChevronLeft, ChevronRight,
  ChevronsUpDown, CircleDot, Clock, Cloud, CloudCog, Code2, Command, Compass, Copy, Cpu, Crown,
  Database, DollarSign, Download, Edit, ExternalLink, Eye, EyeOff, Feather, FileBarChart,
  FileSpreadsheet, FileText, Filter, Flame, FolderTree, FormInput, Forward, Gem, GitBranch, Globe,
  GripVertical, HardDrive, Heart, History, Home, Hourglass, Inbox, Info, Key, KeyRound, Landmark,
  Laptop, LayoutDashboard, LayoutGrid, Layers, Leaf, LifeBuoy, LineChart, Link2, ListChecks,
  Loader2, Lock, LogIn, LogOut, Mail, MapPin, MapPinned, Menu, MessageSquare, Minus, Monitor, MoreHorizontal,
  MoreVertical, Mountain, Network, Package, Palette, Pause, Percent, Phone, PieChart, Play, Plus,
  Printer, Radio, RefreshCw, Reply, Rocket, Save, ScanLine, Search, Send, Server, Settings,
  Shield, ShieldAlert, ShieldCheck, Smartphone, Slash, Sparkles, Square, Star, Table, Target,
  ThumbsUp, Timer, TrendingUp, Trash2, Tv, Unlink, Unlock, Upload, User, UserCheck, UserCog,
  UserPlus, UserX, Users, Wallet, WandSparkles, Waves, Wifi, Workflow, XCircle, Zap,
} from "lucide-react"

/** 规范图标全集(对象字面量键即图标名,与 lucide 导入名一一对应) */
const CANONICAL_ICONS = {
  Activity, AlertTriangle, Anchor, Archive, ArrowDown, ArrowLeft, ArrowRight, ArrowUp, AtSign,
  Award, BadgeCheck, BadgeInfo, Ban, BarChart3, Bell, Bot, Boxes, Building, Building2, Calendar,
  CalendarClock, CalendarDays, Check, CheckCircle2, ChevronDown, ChevronLeft, ChevronRight,
  ChevronsUpDown, CircleDot, Clock, Cloud, CloudCog, Code2, Command, Compass, Copy, Cpu, Crown,
  Database, DollarSign, Download, Edit, ExternalLink, Eye, EyeOff, Feather, FileBarChart,
  FileSpreadsheet, FileText, Filter, Flame, FolderTree, FormInput, Forward, Gem, GitBranch, Globe,
  GripVertical, HardDrive, Heart, History, Home, Hourglass, Inbox, Info, Key, KeyRound, Landmark,
  Laptop, LayoutDashboard, LayoutGrid, Layers, Leaf, LifeBuoy, LineChart, Link2, ListChecks,
  Loader2, Lock, LogIn, LogOut, Mail, MapPin, MapPinned, Menu, MessageSquare, Minus, Monitor, MoreHorizontal,
  MoreVertical, Mountain, Network, Package, Palette, Pause, Percent, Phone, PieChart, Play, Plus,
  Printer, Radio, RefreshCw, Reply, Rocket, Save, ScanLine, Search, Send, Server, Settings,
  Shield, ShieldAlert, ShieldCheck, Smartphone, Slash, Sparkles, Square, Star, Table, Target,
  ThumbsUp, Timer, TrendingUp, Trash2, Tv, Unlink, Unlock, Upload, User, UserCheck, UserCog,
  UserPlus, UserX, Users, Wallet, WandSparkles, Waves, Wifi, Workflow, XCircle, Zap,
}

/** 图标注册表:名称 → lucide 组件(规范名 + 历史别名) */
export const ICON_REGISTRY: Record<string, LucideIcon> = {
  ...CANONICAL_ICONS,
  // 常用别名(契约/UI 中可能出现的历史名称)
  Sitemap: Network,
  Attachment: Link2,
  Flame2: Flame,
  NetworkMap: Network,
  OrgChart: Network,
  Page: FileText,
}

/** 规范图标名列表(供图标库选择器遍历,不含别名避免重复) */
export const ICON_NAMES: string[] = Object.keys(CANONICAL_ICONS)

export interface AppIconProps {
  name?: string | null
  className?: string
  size?: number
}

/** 按名称渲染图标,未知名称回退 LayoutGrid */
export function AppIcon({ name, className, size }: AppIconProps) {
  const Icon = (name && ICON_REGISTRY[name]) || LayoutGrid
  return <Icon className={className} style={size ? { width: size, height: size } : undefined} />
}

export function hasIcon(name?: string | null): boolean {
  return !!name && !!ICON_REGISTRY[name]
}
