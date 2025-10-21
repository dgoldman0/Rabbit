Here’s a complete **example community warren** showing how nested and mixed burrows can form a coherent Rabbit network.

---

# **Example: Willow Glen Community Warren**

## 1. Overview

```
Warren Name: willow-glen
Burrow-ID: ed25519:WGMAIN...
Role: root warren (community)
Members: governance, businesses, families
```

Willow Glen is the *root warren* connecting all local burrows.
Each burrow may host sub-warrens, enabling recursive organization.

---

## 2. Structure Diagram (Conceptual)

```
warren: willow-glen
├── burrow: town-hall
│   ├── /1/agenda
│   ├── /1/regulations
│   └── /q/public-notices
├── burrow: local-market
│   ├── /1/vendors
│   ├── /q/deals
│   └── /u/market-ui
├── warren: oak-family
│   ├── burrow: oak-parent1
│   │   └── /1/photos
│   ├── burrow: oak-parent2
│   │   └── /1/recipes
│   ├── burrow: oak-child1
│   │   └── /1/journal
│   └── burrow: oak-child2
│       └── /1/art
└── warren: pine-family
    ├── burrow: pine-parent
    └── burrow: pine-teen
```

---

## 3. Root Warren Menu (`willow-glen`)

```
1Governance	/1/governance	town-hall	
1Businesses	/1/businesses	local-market	
1Oak Family	/1/oak-family	oak-family	
1Pine Family	/1/pine-family	pine-family	
7Search Warren	/7/search	=	
qCommunity Events	/q/events	=	
```

---

## 4. Governance Burrow (`town-hall`)

### `LIST /1/governance`

```
200 MENU
Length: 142
End:
1Agenda	/1/agenda	=	
1Regulations	/1/regulations	=	
qPublic Notices	/q/public-notices	=	
uUI: Civic Portal	/u/civic-ui	=	
.
```

### Sample Public Notice Stream

```
SUBSCRIBE /q/public-notices
Lane: 5
Txn: PUB1
End:

201 SUBSCRIBED
Lane: 5
Txn: PUB1
End:

EVENT /q/public-notices
Lane: 5
Seq: 12
Length: 36
End:
Town Hall meeting moved to 18:00.
```

---

## 5. Local Business Burrow (`local-market`)

### Menu

```
1Vendors	/1/vendors	=	
qDeals	/q/deals	=	
uUI: Market App	/u/market-ui	=	
```

### Vendor Listing

```
200 MENU
End:
0Bakery	/0/vendors/bakery	=	
0Butcher	/0/vendors/butcher	=	
0Grocer	/0/vendors/grocer	=	
.
```

### Active Deal Event

```
EVENT /q/deals
Lane: 9
Seq: 21
End:
Grocer discount on produce, 10%.
```

---

## 6. Family Warren (`oak-family`)

### `LIST /1/oak-family`

```
200 MENU
End:
1Parent1	/1/parent1	oak-parent1	
1Parent2	/1/parent2	oak-parent2	
1Child1	/1/child1	oak-child1	
1Child2	/1/child2	oak-child2	
qFamily Chat	/q/chat	=	
.
```

### Family Chat Stream

```
SUBSCRIBE /q/chat
Lane: 7
Txn: CHAT1
End:

201 SUBSCRIBED
Lane: 7
Txn: CHAT1
End:

EVENT /q/chat
Lane: 7
Seq: 33
Length: 26
End:
Dinner at 7 tonight?
```

---

## 7. Individual Burrow (`oak-parent1`)

### Menu

```
1Photos	/1/photos	=	
1Documents	/1/docs	=	
uUI: Family Dashboard	/u/dashboard	=	
```

### Sample Fetch

```
FETCH /0/photos/holiday
Lane: 3
Txn: P1
End:

200 CONTENT
Lane: 3
Txn: P1
Length: 44
End:
Image reference: /9/photos/holiday1
```

---

## 8. Sub-Warren Behavior

Each family warren (e.g., `oak-family`) behaves as a **local warren** under `willow-glen`.
Each family member’s burrow can:

* Form direct tunnels to siblings or parents.
* Register its own menu at the family warren.
* Publish events (like chat or reminders) scoped to the sub-warren.

The `oak-family` warren aggregates discovery data:

```
OFFER /warren
Peers: 4
End:

200 PEERS
End:
burrow: oak-parent1
burrow: oak-parent2
burrow: oak-child1
burrow: oak-child2
.
```

---

## 9. Federation Example

A higher-level regional warren (`valley-network`) could reference `willow-glen` as a sub-warren:

```
1Willow Glen Community	/1/willow-glen	willow-glen	
1Lakeside Community	/1/lakeside	lakeside	
.
```

Each community maintains its own nested structure, forming an organically federated topology.

---

## 10. Summary

| Level      | Entity           | Role              | Behavior                         |
| ---------- | ---------------- | ----------------- | -------------------------------- |
| Regional   | `valley-network` | Root federation   | Aggregates communities           |
| Community  | `willow-glen`    | Root local warren | Hosts governance & families      |
| Family     | `oak-family`     | Sub-warren        | Private shared domain            |
| Individual | `oak-parent1`    | Burrow            | Personal space, peer connections |

---

This example illustrates how **Rabbit** natively supports **hierarchical, federated networks** — each burrow can scale from personal use to warren coordination, maintaining the same text-driven, secure, asynchronous protocol model throughout.
