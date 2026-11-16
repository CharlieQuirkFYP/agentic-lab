import React from 'react';
import { Text } from 'react-native';

export function useLocalSearchParams() { return Object.fromEntries(new URLSearchParams(location.search)); }
export function Link({ children, href, ...props }: any) { return <Text {...props} accessibilityRole="link" onPress={() => { location.hash = typeof href === 'string' ? href : href.pathname; }}>{children}</Text>; }
export const router = { back: () => history.back(), push: (href: string) => { location.hash = href; }, replace: (href: string) => { location.hash = href; } };
export function useRouter() { return router; }
